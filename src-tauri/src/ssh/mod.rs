pub mod engine;
#[cfg(test)]
mod engine_integration;
mod forwarding;
mod import;
mod known_hosts;
pub mod model;
pub mod sftp;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rand::RngCore;
#[cfg(windows)]
use russh::keys::agent::client::AgentStream;
use russh::{
    client::{self, AuthResult, Handler, KeyboardInteractiveAuthResponse},
    keys::{agent::client::AgentClient, decode_secret_key, PrivateKeyWithHashAlg},
    ChannelMsg, Disconnect,
};
use secrecy::ExposeSecret;
use serde_json::{Map, Value};
use sha2::{Digest, Sha512};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{copy_bidirectional, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, oneshot},
    time::timeout,
};
use zeroize::Zeroize;

use crate::{
    security::{
        CredentialAddress, CredentialNamespace, CredentialState, SecretState, VaultSecretSelector,
    },
    ssh::{
        engine::{
            HostKeyVerifier, KeyboardInteractiveResponse, PrivateKeyMaterial, RusshEngine,
            SshAuthContext, SshAuthenticator, SshHostKey, SshTarget,
        },
        known_hosts::fingerprint,
        model::{
            AuthMethodRef, HostKeyDecision, HostKeyDecisionRequest, HostKeyPrompt, HostKeyStatus,
            KeepaliveOptions, SshAuthPrompt, SshAuthPromptItem, SshAuthResponseRequest,
            SshConnectRequest, SshError, SshExitEvent, SshForwardingIdRequest, SshForwardingInfo,
            SshForwardingRequest, SshForwardingStatus, SshForwardingType, SshJumpRequest,
            SshOutputEvent, SshResizeRequest, SshSessionIdRequest, SshSessionInfo, SshWriteRequest,
        },
        sftp::{RemoteFileEntry, SftpOverwritePolicy, SftpTransferDescriptor},
    },
};

const CHANNEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub use import::*;
pub use known_hosts::KnownHostsStore;
pub use model::*;

const HOST_KEY_TIMEOUT: Duration = Duration::from_secs(60);
const AUTH_PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BUFFER: usize = 16 * 1024 * 1024;
const VAULT_SECRET_TYPE_PASSPHRASE: &str = "ssh:key-passphrase";

type HostKeySender = oneshot::Sender<HostKeyDecision>;
type AuthSender = oneshot::Sender<Vec<String>>;

#[derive(Clone)]
pub struct SshManager {
    known_hosts: KnownHostsStore,
    sessions: Arc<Mutex<HashMap<String, SshSession>>>,
    host_key_waiters: Arc<Mutex<HashMap<String, HostKeySender>>>,
    auth_waiters: Arc<Mutex<HashMap<String, AuthSender>>>,
    forwardings: Arc<Mutex<HashMap<String, ForwardingRuntime>>>,
    remote_routes: Arc<Mutex<HashMap<(String, String, u32), RemoteForwardRoute>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct SshSession {
    control: mpsc::Sender<SshControl>,
}

struct ForwardingRuntime {
    info: SshForwardingInfo,
    cancel: broadcast::Sender<()>,
}

#[derive(Clone)]
struct RemoteForwardRoute {
    target_address: String,
    target_port: u16,
    cancel: broadcast::Sender<()>,
}

enum SshControl {
    Write(Vec<u8>, oneshot::Sender<Result<(), SshError>>),
    Resize(SshResizeRequest, oneshot::Sender<Result<(), SshError>>),
    OpenDirectTcpip {
        host: String,
        port: u16,
        sender: oneshot::Sender<Result<russh::Channel<client::Msg>, SshError>>,
    },
    OpenSftp {
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    CloseSftp {
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    SftpList {
        path: String,
        sender: oneshot::Sender<Result<Vec<RemoteFileEntry>, SshError>>,
    },
    SftpStat {
        path: String,
        follow: bool,
        sender: oneshot::Sender<Result<RemoteFileEntry, SshError>>,
    },
    SftpMkdir {
        path: String,
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    SftpRename {
        from: String,
        to: String,
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    SftpRemove {
        path: String,
        recursive: bool,
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    SftpOpenUpload {
        path: String,
        size: Option<u64>,
        policy: SftpOverwritePolicy,
        sender: oneshot::Sender<Result<SftpTransferDescriptor, SshError>>,
    },
    SftpOpenDownload {
        path: String,
        sender: oneshot::Sender<Result<SftpTransferDescriptor, SshError>>,
    },
    SftpRead {
        id: String,
        max_bytes: usize,
        sender: oneshot::Sender<Result<(Vec<u8>, SftpTransferDescriptor), SshError>>,
    },
    SftpWrite {
        id: String,
        data: Vec<u8>,
        sender: oneshot::Sender<Result<SftpTransferDescriptor, SshError>>,
    },
    SftpCloseTransfer {
        id: String,
        sender: oneshot::Sender<Result<SftpTransferDescriptor, SshError>>,
    },
    SftpCancelTransfer {
        id: String,
        sender: oneshot::Sender<Result<SftpTransferDescriptor, SshError>>,
    },
    StartRemoteForward {
        bind_host: String,
        bind_port: u16,
        target_address: String,
        target_port: u16,
        cancel: broadcast::Sender<()>,
        sender: oneshot::Sender<Result<u16, SshError>>,
    },
    StopRemoteForward {
        bind_host: String,
        bind_port: u16,
        sender: oneshot::Sender<Result<(), SshError>>,
    },
    Close(oneshot::Sender<Result<(), SshError>>),
}

struct SshHandler {
    manager: SshManager,
    host: String,
    port: u16,
    connection_id: String,
    app: AppHandle,
    host_key_error: Arc<Mutex<Option<SshError>>>,
    remote_routes: Arc<Mutex<HashMap<(String, String, u32), RemoteForwardRoute>>>,
    agent_socket: Option<String>,
    x11_display: Option<String>,
}

impl Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let verifier = ManagerHostKeyVerifier {
            manager: self.manager.clone(),
            app: self.app.clone(),
            connection_id: self.connection_id.clone(),
        };
        let key = match host_key_material(server_public_key) {
            Ok(key) => key,
            Err(error) => {
                *self
                    .host_key_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
                return Ok(false);
            }
        };
        match verifier.verify(&self.host, self.port, &key).await {
            Ok(accepted) => Ok(accepted),
            Err(error) => {
                *self
                    .host_key_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
                Ok(false)
            }
        }
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let route = self
            .remote_routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(
                self.connection_id.clone(),
                connected_address.into(),
                connected_port,
            ))
            .cloned();
        let task = async move {
            let Some(route) = route else {
                let _ = channel.close().await;
                return Ok::<(), russh::Error>(());
            };
            let mut ssh_stream = channel.into_stream();
            let target =
                TcpStream::connect((route.target_address.as_str(), route.target_port)).await;
            let Ok(mut target) = target else {
                let _ = ssh_stream.shutdown().await;
                return Ok::<(), russh::Error>(());
            };
            let mut cancel = route.cancel.subscribe();
            tokio::select! {
                _ = copy_bidirectional(&mut target, &mut ssh_stream) => {}
                _ = cancel.recv() => {
                    let _ = ssh_stream.shutdown().await;
                    let _ = target.shutdown().await;
                }
            }
            Ok::<(), russh::Error>(())
        };
        tokio::spawn(task);
        async { Ok(()) }
    }

    fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let socket = self.agent_socket.clone();
        let task = async move {
            let Ok(mut agent) = connect_agent(socket)
                .await
                .map(|client| client.into_inner())
            else {
                let _ = channel.close().await;
                return Ok::<(), russh::Error>(());
            };
            let mut ssh_stream = channel.into_stream();
            let _ = copy_bidirectional(&mut agent, &mut ssh_stream).await;
            Ok::<(), russh::Error>(())
        };
        tokio::spawn(task);
        async { Ok(()) }
    }

    fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let display = self
            .x11_display
            .clone()
            .or_else(|| std::env::var("DISPLAY").ok())
            .unwrap_or_else(|| ":0".into());
        let task = async move {
            #[cfg(unix)]
            {
                match connect_x11_display(&display).await {
                    Ok(X11Target::Unix(mut target)) => {
                        let mut ssh_stream = channel.into_stream();
                        let _ = copy_bidirectional(&mut target, &mut ssh_stream).await;
                    }
                    Ok(X11Target::Tcp(mut target)) => {
                        let mut ssh_stream = channel.into_stream();
                        let _ = copy_bidirectional(&mut target, &mut ssh_stream).await;
                    }
                    Err(_) => {
                        let _ = channel.close().await;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = display;
                let _ = channel.close().await;
            }
            Ok::<(), russh::Error>(())
        };
        tokio::spawn(task);
        async { Ok(()) }
    }
}

struct ManagerHostKeyVerifier {
    manager: SshManager,
    app: AppHandle,
    connection_id: String,
}

#[async_trait::async_trait]
impl HostKeyVerifier for ManagerHostKeyVerifier {
    async fn verify(&self, host: &str, port: u16, key: &SshHostKey) -> Result<bool, SshError> {
        let public_key = russh::keys::PublicKey::from_openssh(&key.public_key_openssh)
            .map_err(|_| SshError::KeyParse)?;
        self.manager
            .verify_host_key(&self.app, host, port, &self.connection_id, &public_key)
            .await
    }
}

fn host_key_material(key: &russh::keys::PublicKey) -> Result<SshHostKey, SshError> {
    Ok(SshHostKey {
        algorithm: format!("{:?}", key.algorithm()),
        fingerprint_sha256: fingerprint(key),
        public_key_openssh: key.to_openssh().map_err(|_| SshError::KeyParse)?,
    })
}

struct RawSshAuthContext<'a> {
    handle: &'a mut client::Handle<SshHandler>,
}

#[async_trait::async_trait]
impl<'a> SshAuthContext for RawSshAuthContext<'a> {
    async fn authenticate_none(&mut self, username: &str) -> Result<bool, SshError> {
        Ok(matches!(
            self.handle
                .authenticate_none(username)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?,
            AuthResult::Success
        ))
    }

    async fn authenticate_password(
        &mut self,
        username: &str,
        password: &secrecy::SecretString,
    ) -> Result<bool, SshError> {
        Ok(matches!(
            self.handle
                .authenticate_password(username, password.expose_secret())
                .await
                .map_err(|_| SshError::AuthenticationRejected)?,
            AuthResult::Success
        ))
    }

    async fn authenticate_private_key(
        &mut self,
        username: &str,
        key: PrivateKeyMaterial,
    ) -> Result<bool, SshError> {
        let mut text = String::from_utf8(key.openssh.clone()).map_err(|_| SshError::KeyParse)?;
        let passphrase = key.passphrase.as_ref().map(|value| value.expose_secret());
        let private_key = decode_secret_key(&text, passphrase.map(|value| value.as_str()));
        text.zeroize();
        let private_key = private_key.map_err(|_| SshError::KeyParse)?;
        Ok(matches!(
            self.handle
                .authenticate_publickey(
                    username,
                    PrivateKeyWithHashAlg::new(Arc::new(private_key), None),
                )
                .await
                .map_err(|_| SshError::AuthenticationRejected)?,
            AuthResult::Success
        ))
    }

    async fn authenticate_agent(
        &mut self,
        username: &str,
        socket: Option<&str>,
    ) -> Result<bool, SshError> {
        let mut agent = connect_agent(socket.map(str::to_owned)).await?;
        let identities = agent
            .request_identities()
            .await
            .map_err(|_| SshError::AuthenticationRejected)?;
        for identity in identities {
            let result =
                authenticate_publickey_with_agent(self.handle, username, identity, &mut agent)
                    .await
                    .map_err(|_| SshError::AuthenticationRejected)?;
            if matches!(result, AuthResult::Success) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn authenticate_keyboard_interactive_start(
        &mut self,
        username: &str,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        map_keyboard_response(
            self.handle
                .authenticate_keyboard_interactive_start(username, None::<String>)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?,
        )
    }

    async fn authenticate_keyboard_interactive_respond(
        &mut self,
        responses: Vec<String>,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        map_keyboard_response(
            self.handle
                .authenticate_keyboard_interactive_respond(responses)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?,
        )
    }
}

fn map_keyboard_response(
    response: KeyboardInteractiveAuthResponse,
) -> Result<KeyboardInteractiveResponse, SshError> {
    Ok(match response {
        KeyboardInteractiveAuthResponse::Success => KeyboardInteractiveResponse::Success,
        KeyboardInteractiveAuthResponse::Failure { .. } => KeyboardInteractiveResponse::Failure,
        KeyboardInteractiveAuthResponse::InfoRequest {
            name,
            instructions,
            prompts,
        } => KeyboardInteractiveResponse::Prompt(crate::ssh::engine::KeyboardInteractivePrompt {
            name,
            instructions,
            prompts: prompts
                .into_iter()
                .map(|prompt| crate::ssh::engine::KeyboardInteractivePromptItem {
                    text: prompt.prompt,
                    echo: prompt.echo,
                })
                .collect(),
        }),
    })
}

struct ManagerAuthenticator<'a> {
    manager: SshManager,
    app: Option<AppHandle>,
    request: SshConnectRequest,
    secrets: &'a SecretState,
    credentials: &'a CredentialState,
}

#[async_trait::async_trait]
impl SshAuthenticator for ManagerAuthenticator<'_> {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        methods: &[AuthMethodRef],
    ) -> Result<bool, SshError> {
        let methods = if methods.is_empty() {
            &self.request.auth
        } else {
            methods
        };
        if methods.is_empty() {
            return context.authenticate_none(username).await;
        }
        for method in methods {
            let result = match method {
                AuthMethodRef::Password { secret_ref } => {
                    let password = resolve_secret_ref(secret_ref, self.secrets, self.credentials)?;
                    context.authenticate_password(username, &password).await
                }
                AuthMethodRef::PrivateKey {
                    file_ref,
                    passphrase_ref,
                } => {
                    let material = self
                        .manager
                        .load_private_key_material(
                            file_ref,
                            passphrase_ref.as_deref(),
                            self.secrets,
                            self.credentials,
                        )
                        .await?;
                    match context.authenticate_private_key(username, material).await {
                        Err(SshError::KeyParse) if passphrase_ref.is_none() => {
                            let responses = self
                                .manager
                                .prompt_for_responses(
                                    self.app.as_ref().ok_or(SshError::Closed)?,
                                    SshAuthPrompt {
                                        request_id: String::new(),
                                        id: self.request.profile_id.clone(),
                                        connection_id: self
                                            .request
                                            .connection_id
                                            .clone()
                                            .unwrap_or_else(|| self.request.profile_id.clone()),
                                        name: "Private key passphrase".into(),
                                        instructions: "The private key is encrypted.".into(),
                                        prompts: vec![SshAuthPromptItem {
                                            text: "Passphrase".into(),
                                            echo: false,
                                        }],
                                    },
                                )
                                .await?;
                            let mut responses = responses;
                            let result = if let Some(passphrase) = responses.first() {
                                context
                                    .authenticate_private_key(
                                        username,
                                        PrivateKeyMaterial {
                                            openssh: self
                                                .manager
                                                .load_private_key_bytes(file_ref, self.secrets)
                                                .await?,
                                            passphrase: Some(secrecy::SecretString::new(
                                                passphrase.clone(),
                                            )),
                                        },
                                    )
                                    .await
                            } else {
                                Err(SshError::AuthenticationRejected)
                            };
                            responses.zeroize();
                            result
                        }
                        result => result,
                    }
                }
                AuthMethodRef::Agent { socket } => {
                    context
                        .authenticate_agent(username, socket.as_deref())
                        .await
                }
                AuthMethodRef::KeyboardInteractive => {
                    let mut response = context
                        .authenticate_keyboard_interactive_start(username)
                        .await?;
                    loop {
                        match response {
                            KeyboardInteractiveResponse::Success => break Ok(true),
                            KeyboardInteractiveResponse::Failure => break Ok(false),
                            KeyboardInteractiveResponse::Prompt(prompt) => {
                                let mut responses = self
                                    .manager
                                    .prompt_for_responses(
                                        self.app.as_ref().ok_or(SshError::Closed)?,
                                        SshAuthPrompt {
                                            request_id: String::new(),
                                            id: self.request.profile_id.clone(),
                                            connection_id: self
                                                .request
                                                .connection_id
                                                .clone()
                                                .unwrap_or_else(|| self.request.profile_id.clone()),
                                            name: prompt.name,
                                            instructions: prompt.instructions,
                                            prompts: prompt
                                                .prompts
                                                .into_iter()
                                                .map(|prompt| SshAuthPromptItem {
                                                    text: prompt.text,
                                                    echo: prompt.echo,
                                                })
                                                .collect(),
                                        },
                                    )
                                    .await?;
                                let result = context
                                    .authenticate_keyboard_interactive_respond(responses.clone())
                                    .await;
                                responses.zeroize();
                                response = result?;
                            }
                        }
                    }
                }
            }?;
            if result {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

async fn disconnect_jump_handles(handles: &mut Vec<client::Handle<SshHandler>>) {
    for handle in handles.drain(..) {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "connection setup failed", "")
            .await;
    }
}

async fn disconnect_connection(
    handle: &mut client::Handle<SshHandler>,
    jump_handles: &mut Vec<client::Handle<SshHandler>>,
    reason: Disconnect,
    description: &'static str,
) {
    let _ = handle.disconnect(reason, description, "").await;
    disconnect_jump_handles(jump_handles).await;
}

impl SshManager {
    pub fn new(known_hosts_path: std::path::PathBuf) -> Self {
        Self {
            known_hosts: KnownHostsStore::new(known_hosts_path),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            host_key_waiters: Arc::new(Mutex::new(HashMap::new())),
            auth_waiters: Arc::new(Mutex::new(HashMap::new())),
            forwardings: Arc::new(Mutex::new(HashMap::new())),
            remote_routes: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    fn handler(
        &self,
        app: &AppHandle,
        request: &SshConnectRequest,
        host: String,
        port: u16,
        connection_id: &str,
        host_key_error: Arc<Mutex<Option<SshError>>>,
    ) -> SshHandler {
        SshHandler {
            manager: self.clone(),
            host,
            port,
            connection_id: connection_id.into(),
            app: app.clone(),
            host_key_error,
            remote_routes: Arc::clone(&self.remote_routes),
            agent_socket: request.auth.iter().find_map(|method| match method {
                AuthMethodRef::Agent { socket } => socket.clone(),
                _ => None,
            }),
            x11_display: request.x11_display.clone(),
        }
    }

    async fn connect_direct(
        &self,
        config: Arc<client::Config>,
        app: &AppHandle,
        request: &SshConnectRequest,
        connection_id: &str,
    ) -> Result<client::Handle<SshHandler>, SshError> {
        let host_key_error = Arc::new(Mutex::new(None));
        let handler = self.handler(
            app,
            request,
            request.host.clone(),
            request.port,
            connection_id,
            Arc::clone(&host_key_error),
        );
        match timeout(
            Duration::from_secs(30),
            client::connect(config, (request.host.clone(), request.port), handler),
        )
        .await
        {
            Err(_) => Err(SshError::Timeout),
            Ok(Ok(handle)) => Ok(handle),
            Ok(Err(_)) => Err(host_key_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .unwrap_or(SshError::Connection)),
        }
    }

    async fn connect_direct_engine(
        &self,
        config: Arc<client::Config>,
        app: &AppHandle,
        request: &SshConnectRequest,
        connection_id: &str,
        secrets: &SecretState,
        credentials: &CredentialState,
    ) -> Result<client::Handle<SshHandler>, SshError> {
        let host_key_error = Arc::new(Mutex::new(None));
        let handler = self.handler(
            app,
            request,
            request.host.clone(),
            request.port,
            connection_id,
            Arc::clone(&host_key_error),
        );
        let authenticator = Arc::new(ManagerAuthenticator {
            manager: self.clone(),
            app: Some(app.clone()),
            request: request.clone(),
            secrets,
            credentials,
        });
        let engine = RusshEngine::from_shared_config(config, Duration::from_secs(30));
        engine
            .connect_with_handler(
                &SshTarget {
                    host: request.host.clone(),
                    port: request.port,
                    username: request.username.clone().unwrap_or_else(|| "root".into()),
                },
                handler,
                host_key_error,
                authenticator.as_ref(),
            )
            .await
    }

    async fn connect_over_channel(
        &self,
        config: Arc<client::Config>,
        app: &AppHandle,
        request: &SshConnectRequest,
        connection_id: &str,
        channel: russh::Channel<client::Msg>,
    ) -> Result<client::Handle<SshHandler>, SshError> {
        let host_key_error = Arc::new(Mutex::new(None));
        let handler = self.handler(
            app,
            request,
            request.host.clone(),
            request.port,
            connection_id,
            Arc::clone(&host_key_error),
        );
        match timeout(
            Duration::from_secs(30),
            client::connect_stream(config, channel.into_stream(), handler),
        )
        .await
        {
            Err(_) => Err(SshError::Timeout),
            Ok(Ok(handle)) => Ok(handle),
            Ok(Err(_)) => Err(host_key_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
                .unwrap_or(SshError::Connection)),
        }
    }

    pub async fn connect(
        &self,
        app: AppHandle,
        request: SshConnectRequest,
        secrets: Arc<SecretState>,
        credentials: CredentialState,
    ) -> Result<SshSessionInfo, SshError> {
        validate_request(&request)?;
        let username = request.username.clone().unwrap_or_else(|| "root".into());
        let connection_id = request
            .connection_id
            .clone()
            .unwrap_or_else(|| request.profile_id.clone());
        let config = client::Config {
            inactivity_timeout: request.keepalive.as_ref().map(|options| {
                Duration::from_millis(
                    options
                        .interval_ms
                        .saturating_mul((options.max_count as u64).max(1)),
                )
            }),
            keepalive_interval: request
                .keepalive
                .as_ref()
                .map(|options| Duration::from_millis(options.interval_ms)),
            keepalive_max: request
                .keepalive
                .as_ref()
                .map(|options| options.max_count as usize)
                .unwrap_or(3),
            ..Default::default()
        };
        let config = Arc::new(config);
        let mut jump_handles = Vec::new();
        let mut handle;
        if request.jump_chain.is_empty() {
            handle = self
                .connect_direct_engine(
                    Arc::clone(&config),
                    &app,
                    &request,
                    &connection_id,
                    &secrets,
                    &credentials,
                )
                .await?;
        } else {
            let first = jump_request(&request, &request.jump_chain[0], &connection_id, 0);
            handle = self
                .connect_direct(Arc::clone(&config), &app, &first, &connection_id)
                .await?;
            let first_username = first.username.clone().unwrap_or_else(|| "root".into());
            let first_authenticated = match self
                .authenticate(
                    &app,
                    &mut handle,
                    &first,
                    &first_username,
                    &secrets,
                    &credentials,
                )
                .await
            {
                Ok(authenticated) => authenticated,
                Err(error) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "authentication failed",
                    )
                    .await;
                    return Err(error);
                }
            };
            if !first_authenticated {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::AuthCancelledByUser,
                    "authentication rejected",
                )
                .await;
                return Err(SshError::AuthenticationRejected);
            }
            for (index, hop) in request.jump_chain.iter().enumerate().skip(1) {
                let next = jump_request(&request, hop, &connection_id, index);
                let channel = match handle
                    .channel_open_direct_tcpip(&next.host, u32::from(next.port), "127.0.0.1", 0)
                    .await
                {
                    Ok(channel) => channel,
                    Err(_) => {
                        disconnect_connection(
                            &mut handle,
                            &mut jump_handles,
                            Disconnect::ByApplication,
                            "jump channel open failed",
                        )
                        .await;
                        return Err(SshError::ChannelOpen);
                    }
                };
                jump_handles.push(handle);
                handle = match self
                    .connect_over_channel(Arc::clone(&config), &app, &next, &connection_id, channel)
                    .await
                {
                    Ok(handle) => handle,
                    Err(error) => {
                        disconnect_jump_handles(&mut jump_handles).await;
                        return Err(error);
                    }
                };
                let next_username = next.username.clone().unwrap_or_else(|| "root".into());
                let next_authenticated = match self
                    .authenticate(
                        &app,
                        &mut handle,
                        &next,
                        &next_username,
                        &secrets,
                        &credentials,
                    )
                    .await
                {
                    Ok(authenticated) => authenticated,
                    Err(error) => {
                        disconnect_connection(
                            &mut handle,
                            &mut jump_handles,
                            Disconnect::ByApplication,
                            "authentication failed",
                        )
                        .await;
                        return Err(error);
                    }
                };
                if !next_authenticated {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::AuthCancelledByUser,
                        "authentication rejected",
                    )
                    .await;
                    return Err(SshError::AuthenticationRejected);
                }
            }
            let channel = match handle
                .channel_open_direct_tcpip(&request.host, u32::from(request.port), "127.0.0.1", 0)
                .await
            {
                Ok(channel) => channel,
                Err(_) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "target channel open failed",
                    )
                    .await;
                    return Err(SshError::ChannelOpen);
                }
            };
            jump_handles.push(handle);
            handle = match self
                .connect_over_channel(Arc::clone(&config), &app, &request, &connection_id, channel)
                .await
            {
                Ok(handle) => handle,
                Err(error) => {
                    disconnect_jump_handles(&mut jump_handles).await;
                    return Err(error);
                }
            };
            let target_authenticated = match self
                .authenticate(
                    &app,
                    &mut handle,
                    &request,
                    &username,
                    &secrets,
                    &credentials,
                )
                .await
            {
                Ok(authenticated) => authenticated,
                Err(error) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "authentication failed",
                    )
                    .await;
                    return Err(error);
                }
            };
            if !target_authenticated {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::AuthCancelledByUser,
                    "authentication rejected",
                )
                .await;
                return Err(SshError::AuthenticationRejected);
            }
        }

        let channel = match handle.channel_open_session().await {
            Ok(channel) => channel,
            Err(_) => {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "session channel open failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
        };
        let mut channel = channel;
        let mut pending = VecDeque::new();
        let terminal = &request.terminal;
        if channel
            .request_pty(
                true,
                &terminal.term,
                terminal.columns,
                terminal.rows,
                terminal.pixel_width.unwrap_or_default(),
                terminal.pixel_height.unwrap_or_default(),
                &[],
            )
            .await
            .is_err()
        {
            disconnect_connection(
                &mut handle,
                &mut jump_handles,
                Disconnect::ByApplication,
                "PTY request failed",
            )
            .await;
            return Err(SshError::ChannelOpen);
        }
        match wait_for_channel_confirmation(&mut channel).await {
            Ok(messages) => pending.extend(messages),
            Err(_) => {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "PTY request confirmation failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
        }
        for (name, value) in &request.environment {
            if channel.set_env(true, name, value).await.is_err() {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "environment setup failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
            match wait_for_channel_confirmation(&mut channel).await {
                Ok(messages) => pending.extend(messages),
                Err(_) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "environment setup confirmation failed",
                    )
                    .await;
                    return Err(SshError::ChannelOpen);
                }
            }
        }
        if request.agent_forward {
            if channel.agent_forward(true).await.is_err() {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "agent forwarding setup failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
            match wait_for_channel_confirmation(&mut channel).await {
                Ok(messages) => pending.extend(messages),
                Err(_) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "agent forwarding confirmation failed",
                    )
                    .await;
                    return Err(SshError::ChannelOpen);
                }
            }
        }
        if request.x11 {
            let display = request
                .x11_display
                .clone()
                .or_else(|| std::env::var("DISPLAY").ok())
                .unwrap_or_else(|| ":0".into());
            let cookie = x11_cookie(&display);
            if channel
                .request_x11(true, false, "MIT-MAGIC-COOKIE-1", cookie, 0)
                .await
                .is_err()
            {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "X11 forwarding setup failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
            match wait_for_channel_confirmation(&mut channel).await {
                Ok(messages) => pending.extend(messages),
                Err(_) => {
                    disconnect_connection(
                        &mut handle,
                        &mut jump_handles,
                        Disconnect::ByApplication,
                        "X11 forwarding confirmation failed",
                    )
                    .await;
                    return Err(SshError::ChannelOpen);
                }
            }
        }
        if channel.request_shell(true).await.is_err() {
            disconnect_connection(
                &mut handle,
                &mut jump_handles,
                Disconnect::ByApplication,
                "shell request failed",
            )
            .await;
            return Err(SshError::ChannelOpen);
        }

        match wait_for_channel_confirmation(&mut channel).await {
            Ok(messages) => pending.extend(messages),
            Err(_) => {
                disconnect_connection(
                    &mut handle,
                    &mut jump_handles,
                    Disconnect::ByApplication,
                    "shell request confirmation failed",
                )
                .await;
                return Err(SshError::ChannelOpen);
            }
        }

        let id = self.new_id("session");
        let (mut reader, writer) = channel.split();
        let (control, mut controls) = mpsc::channel(32);
        let sessions = Arc::clone(&self.sessions);
        let task_id = id.clone();
        let task_connection_id = connection_id.clone();
        let task_profile_id = request.profile_id.clone();
        let task_app = app.clone();
        let task_manager = self.clone();
        let mut jump_handles = jump_handles;
        let task_id_for_task = task_id.clone();
        tauri::async_runtime::spawn(async move {
            let mut pending = pending;
            let mut sftp = None;
            let mut exit_event_emitted = false;
            loop {
                tokio::select! {
                    message = async {
                        if let Some(message) = pending.pop_front() {
                            Some(message)
                        } else {
                            reader.wait().await
                        }
                    } => {
                        let Some(message) = message else { break };
                        let is_exit_message = matches!(
                            &message,
                            ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. }
                        );
                        if !emit_channel_message(
                            &task_app,
                            &task_id_for_task,
                            &task_connection_id,
                            &task_profile_id,
                            message,
                        ) {
                            break;
                        }
                        exit_event_emitted |= is_exit_message;
                    }
                    control_message = controls.recv() => {
                        let Some(control_message) = control_message else { break };
                        if !handle_control(
                            &mut handle,
                            &writer,
                            &mut sftp,
                            &task_connection_id,
                            &task_manager.remote_routes,
                            control_message,
                        ).await {
                            break;
                        }
                    }
                }
            }
            if let Some(manager) = sftp {
                manager.shutdown().await;
            }
            sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&task_id_for_task);
            if !exit_event_emitted {
                let _ = task_app.emit(
                    "ssh:exit",
                    SshExitEvent {
                        id: task_id_for_task.clone(),
                        connection_id: task_connection_id.clone(),
                        profile_id: task_profile_id,
                        exit_code: None,
                        signal: None,
                    },
                );
            }
            task_manager.stop_forwardings_for_session(&task_id_for_task, &task_connection_id);
            let _ = handle
                .disconnect(Disconnect::ByApplication, "session closed", "")
                .await;
            for jump_handle in jump_handles.drain(..) {
                let _ = jump_handle
                    .disconnect(Disconnect::ByApplication, "session closed", "")
                    .await;
            }
        });

        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), SshSession { control });
        Ok(SshSessionInfo {
            id,
            profile_id: request.profile_id,
            host: request.host,
            port: request.port,
            username,
        })
    }

    pub async fn host_key_decision(&self, request: HostKeyDecisionRequest) -> Result<(), SshError> {
        let sender = self
            .host_key_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&request.request_id)
            .ok_or_else(|| {
                SshError::InvalidRequest("host key request is unknown or expired".into())
            })?;
        sender.send(request.decision).map_err(|_| SshError::Closed)
    }

    pub async fn auth_response(&self, request: SshAuthResponseRequest) -> Result<(), SshError> {
        if request.responses.len() > 32
            || request
                .responses
                .iter()
                .any(|value| value.len() > 64 * 1024)
        {
            return Err(SshError::InvalidRequest(
                "authentication response is too large".into(),
            ));
        }
        let sender = self
            .auth_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&request.request_id)
            .ok_or_else(|| {
                SshError::InvalidRequest("authentication request is unknown or expired".into())
            })?;
        sender.send(request.responses).map_err(|_| SshError::Closed)
    }

    pub async fn write(&self, request: SshWriteRequest) -> Result<(), SshError> {
        if request.data.len() > MAX_BUFFER {
            return Err(SshError::InvalidRequest("SSH write is too large".into()));
        }
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::Write(request.data, sender))
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn resize(&self, request: SshResizeRequest) -> Result<(), SshError> {
        if request.columns == 0
            || request.rows == 0
            || request.columns > 1000
            || request.rows > 1000
        {
            return Err(SshError::InvalidRequest(
                "terminal dimensions are invalid".into(),
            ));
        }
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::Resize(request, sender))
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn sftp_open(
        &self,
        request: SshSessionIdRequest,
    ) -> Result<sftp::SftpSessionInfo, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::OpenSftp { sender })
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(sftp::SftpSessionInfo {
            id: request.id.clone(),
            ssh_session_id: request.id,
        })
    }

    pub async fn sftp_close(&self, request: SshSessionIdRequest) -> Result<(), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::CloseSftp { sender })
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn sftp_list(
        &self,
        request: sftp::SftpPathRequest,
    ) -> Result<Vec<RemoteFileEntry>, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpList {
                path: request.path,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_stat(
        &self,
        request: sftp::SftpStatRequest,
    ) -> Result<RemoteFileEntry, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpStat {
                path: request.path,
                follow: request.follow,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_mkdir(&self, request: sftp::SftpPathRequest) -> Result<(), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpMkdir {
                path: request.path,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn sftp_rename(&self, request: sftp::SftpRenameRequest) -> Result<(), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpRename {
                from: request.from,
                to: request.to,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn sftp_remove(&self, request: sftp::SftpRemoveRequest) -> Result<(), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpRemove {
                path: request.path,
                recursive: request.recursive,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn sftp_open_upload(
        &self,
        request: sftp::SftpUploadOpenRequest,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpOpenUpload {
                path: request.path,
                size: request.size,
                policy: request.overwrite_policy,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_open_download(
        &self,
        request: sftp::SftpDownloadOpenRequest,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpOpenDownload {
                path: request.path,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_read(
        &self,
        request: sftp::SftpReadRequest,
    ) -> Result<(Vec<u8>, SftpTransferDescriptor), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpRead {
                id: request.transfer_id,
                max_bytes: request.max_bytes,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_write(
        &self,
        request: sftp::SftpWriteRequest,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpWrite {
                id: request.transfer_id,
                data: request.data,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_close_transfer(
        &self,
        request: sftp::SftpTransferIdRequest,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpCloseTransfer {
                id: request.transfer_id,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn sftp_cancel_transfer(
        &self,
        request: sftp::SftpTransferIdRequest,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::SftpCancelTransfer {
                id: request.transfer_id,
                sender,
            })
            .await
            .map_err(|_| SshError::Closed)?;
        Ok(receiver.await.map_err(|_| SshError::Closed)??)
    }

    pub async fn close(&self, request: SshSessionIdRequest) -> Result<(), SshError> {
        let session = self.session(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        session
            .control
            .send(SshControl::Close(sender))
            .await
            .map_err(|_| SshError::Closed)?;
        receiver.await.map_err(|_| SshError::Closed)??;
        Ok(())
    }

    pub async fn start_forwarding(
        &self,
        app: AppHandle,
        request: SshForwardingRequest,
    ) -> Result<SshForwardingInfo, SshError> {
        validate_forwarding_request(&request)?;
        let session = self.session(&request.session_id)?;
        let id = self.new_id("forward");
        let (cancel, _) = broadcast::channel(4);
        let mut info = SshForwardingInfo {
            id: id.clone(),
            session_id: request.session_id.clone(),
            kind: request.kind,
            bind_host: request.bind_host.clone(),
            bind_port: request.bind_port,
            target_address: request.target_address.clone(),
            target_port: request.target_port,
            status: SshForwardingStatus::Starting,
            last_error: None,
        };
        self.forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                id.clone(),
                ForwardingRuntime {
                    info: info.clone(),
                    cancel: cancel.clone(),
                },
            );
        emit_forwarding(&app, &info);

        match request.kind {
            SshForwardingType::Local | SshForwardingType::Dynamic => {
                let listener = match TcpListener::bind((
                    request.bind_host.as_str(),
                    request.bind_port,
                ))
                .await
                {
                    Ok(listener) => listener,
                    Err(error) => {
                        self.fail_forwarding(&app, &id, error.to_string());
                        return Err(SshError::Connection);
                    }
                };
                info.bind_port = listener
                    .local_addr()
                    .map_err(|_| SshError::Connection)?
                    .port();
                info.status = SshForwardingStatus::Active;
                self.update_forwarding(info.clone());
                emit_forwarding(&app, &info);
                let manager = self.clone();
                tokio::spawn(run_local_forward(
                    manager,
                    app,
                    info.clone(),
                    listener,
                    session.control,
                    cancel,
                ));
            }
            SshForwardingType::Remote => {
                let (sender, receiver) = oneshot::channel();
                session
                    .control
                    .send(SshControl::StartRemoteForward {
                        bind_host: request.bind_host.clone(),
                        bind_port: request.bind_port,
                        target_address: request.target_address.clone(),
                        target_port: request.target_port,
                        cancel,
                        sender,
                    })
                    .await
                    .map_err(|_| SshError::Closed)?;
                let port = match receiver.await.map_err(|_| SshError::Closed)? {
                    Ok(port) => port,
                    Err(error) => {
                        self.fail_forwarding(&app, &id, error.to_string());
                        return Err(error);
                    }
                };
                info.bind_port = port;
                info.status = SshForwardingStatus::Active;
                self.update_forwarding(info.clone());
                emit_forwarding(&app, &info);
            }
        }
        Ok(info)
    }

    pub async fn stop_forwarding(
        &self,
        app: AppHandle,
        request: SshForwardingIdRequest,
    ) -> Result<(), SshError> {
        let runtime = self
            .forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request.id)
            .ok_or_else(|| SshError::InvalidRequest("forwarding is unknown or closed".into()))?;
        let mut info = runtime.info;
        info.status = SshForwardingStatus::Stopping;
        emit_forwarding(&app, &info);
        let _ = runtime.cancel.send(());
        if info.kind == SshForwardingType::Remote {
            let session = self.session(&info.session_id)?;
            let (sender, receiver) = oneshot::channel();
            session
                .control
                .send(SshControl::StopRemoteForward {
                    bind_host: info.bind_host.clone(),
                    bind_port: info.bind_port,
                    sender,
                })
                .await
                .map_err(|_| SshError::Closed)?;
            receiver.await.map_err(|_| SshError::Closed)??;
        }
        info.status = SshForwardingStatus::Stopped;
        emit_forwarding(&app, &info);
        Ok(())
    }

    pub fn list_forwardings(&self) -> Vec<SshForwardingInfo> {
        self.forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|runtime| runtime.info.clone())
            .collect()
    }

    fn update_forwarding(&self, info: SshForwardingInfo) {
        if let Some(runtime) = self
            .forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&info.id)
        {
            runtime.info = info;
        }
    }

    fn fail_forwarding(&self, app: &AppHandle, id: &str, error: String) {
        if let Some(runtime) = self
            .forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(id)
        {
            runtime.info.status = SshForwardingStatus::Failed;
            runtime.info.last_error = Some(error);
            emit_forwarding(app, &runtime.info);
        }
    }

    fn finish_forwarding(&self, app: &AppHandle, id: &str) {
        let Some(runtime) = self
            .forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
        else {
            return;
        };
        let mut info = runtime.info;
        info.status = SshForwardingStatus::Stopped;
        emit_forwarding(app, &info);
    }

    fn stop_forwardings_for_session(&self, session_id: &str, connection_id: &str) {
        let mut forwardings = self
            .forwardings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ids: Vec<_> = forwardings
            .iter()
            .filter(|(_, runtime)| runtime.info.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(runtime) = forwardings.remove(&id) {
                let _ = runtime.cancel.send(());
            }
        }
        self.remote_routes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(route_connection_id, _, _), route| {
                if route_connection_id == connection_id {
                    let _ = route.cancel.send(());
                    false
                } else {
                    true
                }
            });
    }

    async fn verify_host_key(
        &self,
        app: &AppHandle,
        host: &str,
        port: u16,
        connection_id: &str,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, SshError> {
        let classification = self.known_hosts.classify(host, port, key)?;
        let Some((status, previous_fingerprints)) = classification else {
            return Ok(true);
        };
        let request_id = self.new_id("host-key");
        let (sender, receiver) = oneshot::channel();
        self.host_key_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(request_id.clone(), sender);
        if app
            .emit(
                "ssh:hostKeyPrompt",
                HostKeyPrompt {
                    request_id: request_id.clone(),
                    connection_id: connection_id.into(),
                    host: host.into(),
                    port,
                    algorithm: format!("{:?}", key.algorithm()),
                    fingerprint_sha256: fingerprint(key),
                    status,
                    previous_fingerprints,
                },
            )
            .is_err()
        {
            self.host_key_waiters
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&request_id);
            return Err(SshError::HostKeyRejected);
        }
        let decision = timeout(HOST_KEY_TIMEOUT, receiver).await;
        self.host_key_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&request_id);
        let Ok(Ok(decision)) = decision else {
            return Err(SshError::Timeout);
        };
        match host_key_decision_action(status, decision)? {
            HostKeyDecisionAction::AcceptOnce => Ok(true),
            HostKeyDecisionAction::Save => self.known_hosts.save(host, port, key).map(|_| true),
        }
    }

    async fn authenticate(
        &self,
        app: &AppHandle,
        handle: &mut client::Handle<SshHandler>,
        request: &SshConnectRequest,
        username: &str,
        secrets: &SecretState,
        credentials: &CredentialState,
    ) -> Result<bool, SshError> {
        let authenticator = ManagerAuthenticator {
            manager: self.clone(),
            app: Some(app.clone()),
            request: request.clone(),
            secrets,
            credentials,
        };
        let mut context = RawSshAuthContext { handle };
        authenticator
            .authenticate(&mut context, username, &request.auth)
            .await
    }

    async fn prompt_for_responses(
        &self,
        app: &AppHandle,
        mut prompt: SshAuthPrompt,
    ) -> Result<Vec<String>, SshError> {
        let request_id = self.new_id("auth");
        prompt.request_id = request_id.clone();
        let (sender, receiver) = oneshot::channel();
        self.auth_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(request_id.clone(), sender);
        if app.emit("ssh:authPrompt", prompt).is_err() {
            self.auth_waiters
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&request_id);
            return Err(SshError::Closed);
        }
        let result = timeout(AUTH_PROMPT_TIMEOUT, receiver).await;
        self.auth_waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&request_id);
        match result {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(SshError::Closed),
            Err(_) => Err(SshError::Timeout),
        }
    }

    async fn load_private_key_material(
        &self,
        file_ref: &str,
        passphrase_ref: Option<&str>,
        secrets: &SecretState,
        credentials: &CredentialState,
    ) -> Result<PrivateKeyMaterial, SshError> {
        let openssh = self.load_private_key_bytes(file_ref, secrets).await?;
        let passphrase = match passphrase_ref {
            Some(reference) => Some(resolve_secret_ref(reference, secrets, credentials)?),
            None => resolve_saved_private_key_passphrase(&openssh, secrets, credentials),
        };
        Ok(PrivateKeyMaterial {
            openssh,
            passphrase,
        })
    }

    async fn load_private_key_bytes(
        &self,
        file_ref: &str,
        secrets: &SecretState,
    ) -> Result<Vec<u8>, SshError> {
        let mut bytes = if let Some(id) = file_ref.strip_prefix("vault://") {
            secrets.get_file(id).map_err(|_| SshError::KeyParse)?
        } else {
            fs::read(file_ref).map_err(|_| SshError::KeyParse)?
        };
        if bytes.len() > MAX_BUFFER {
            bytes.zeroize();
            return Err(SshError::KeyParse);
        }
        Ok(bytes)
    }

    fn session(&self, id: &str) -> Result<SshSession, SshError> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| SshError::InvalidRequest("SSH session is unknown or closed".into()))
    }

    fn new_id(&self, prefix: &str) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("ssh-{prefix}-{id}")
    }
}

async fn handle_control(
    handle: &mut client::Handle<SshHandler>,
    writer: &russh::ChannelWriteHalf<client::Msg>,
    sftp: &mut Option<sftp::SftpManager>,
    connection_id: &str,
    remote_routes: &Arc<Mutex<HashMap<(String, String, u32), RemoteForwardRoute>>>,
    control: SshControl,
) -> bool {
    match control {
        SshControl::Write(data, sender) => {
            let mut sink = writer.make_writer();
            let result = match sink.write_all(&data).await {
                Ok(()) => sink.flush().await.map_err(|_| SshError::Closed),
                Err(_) => Err(SshError::Closed),
            };
            let keep_running = result.is_ok();
            let _ = sender.send(result);
            keep_running
        }
        SshControl::Resize(request, sender) => {
            let result = writer
                .window_change(
                    request.columns,
                    request.rows,
                    request.pixel_width.unwrap_or_default(),
                    request.pixel_height.unwrap_or_default(),
                )
                .await
                .map_err(|_| SshError::Closed);
            let keep_running = result.is_ok();
            let _ = sender.send(result);
            keep_running
        }
        SshControl::OpenDirectTcpip { host, port, sender } => {
            let result = handle
                .channel_open_direct_tcpip(host, u32::from(port), "127.0.0.1", 0)
                .await
                .map_err(|_| SshError::ChannelOpen);
            let _ = sender.send(result);
            true
        }
        SshControl::OpenSftp { sender } => {
            let result = async {
                if sftp.is_some() {
                    return Ok(());
                }
                let channel = handle
                    .channel_open_session()
                    .await
                    .map_err(|_| SshError::ChannelOpen)?;
                channel
                    .request_subsystem(true, "sftp")
                    .await
                    .map_err(|_| SshError::ChannelOpen)?;
                let session = russh_sftp::client::SftpSession::new(channel.into_stream())
                    .await
                    .map_err(|error| SshError::Sftp(error.to_string()))?;
                *sftp = Some(sftp::SftpManager::new(session));
                Ok(())
            }
            .await;
            let _ = sender.send(result);
            true
        }
        SshControl::CloseSftp { sender } => {
            let result = async {
                if let Some(manager) = sftp.take() {
                    manager.shutdown().await;
                }
                Ok(())
            }
            .await;
            let _ = sender.send(result);
            true
        }
        SshControl::SftpList { path, sender } => {
            let result = match sftp.as_ref() {
                Some(manager) => manager.list(&path).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpStat {
            path,
            follow,
            sender,
        } => {
            let result = match sftp.as_ref() {
                Some(manager) => manager.stat(&path, follow).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpMkdir { path, sender } => {
            let result = match sftp.as_ref() {
                Some(manager) => manager.mkdir(&path).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpRename { from, to, sender } => {
            let result = match sftp.as_ref() {
                Some(manager) => manager.rename(&from, &to).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpRemove {
            path,
            recursive,
            sender,
        } => {
            let result = match sftp.as_ref() {
                Some(manager) => manager.remove(&path, recursive).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpOpenUpload {
            path,
            size,
            policy,
            sender,
        } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.open_upload(&path, size, policy).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpOpenDownload { path, sender } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.open_download(&path).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpRead {
            id,
            max_bytes,
            sender,
        } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.read(&id, max_bytes).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpWrite { id, data, sender } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.write(&id, &data).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpCloseTransfer { id, sender } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.close(&id).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::SftpCancelTransfer { id, sender } => {
            let result = match sftp.as_mut() {
                Some(manager) => manager.cancel(&id).await,
                None => Err(SshError::InvalidRequest("SFTP is not open".into())),
            };
            let _ = sender.send(result);
            true
        }
        SshControl::StartRemoteForward {
            bind_host,
            bind_port,
            target_address,
            target_port,
            cancel,
            sender,
        } => {
            let result = handle
                .tcpip_forward(bind_host.clone(), u32::from(bind_port))
                .await
                .map_err(|_| SshError::ChannelOpen);
            if let Ok(port) = result {
                remote_routes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(
                        (connection_id.into(), bind_host, u32::from(port)),
                        RemoteForwardRoute {
                            target_address,
                            target_port,
                            cancel,
                        },
                    );
                let _ = sender.send(Ok(port as u16));
            } else {
                let _ = sender.send(result.map(|port| port as u16));
            }
            true
        }
        SshControl::StopRemoteForward {
            bind_host,
            bind_port,
            sender,
        } => {
            let result = handle
                .cancel_tcpip_forward(bind_host.clone(), u32::from(bind_port))
                .await
                .map_err(|_| SshError::Closed);
            if result.is_ok() {
                if let Some(route) = remote_routes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&(connection_id.into(), bind_host, u32::from(bind_port)))
                {
                    let _ = route.cancel.send(());
                }
            }
            let _ = sender.send(result);
            true
        }
        SshControl::Close(sender) => {
            if let Some(manager) = sftp.take() {
                manager.shutdown().await;
            }
            let result = match writer.close().await {
                Ok(()) => handle
                    .disconnect(Disconnect::ByApplication, "closed by user", "")
                    .await
                    .map_err(|_| SshError::Closed),
                Err(_) => Err(SshError::Closed),
            };
            let _ = sender.send(result);
            false
        }
    }
}

async fn run_local_forward(
    manager: SshManager,
    app: AppHandle,
    info: SshForwardingInfo,
    listener: TcpListener,
    control: mpsc::Sender<SshControl>,
    cancel: broadcast::Sender<()>,
) {
    let mut cancellation = cancel.subscribe();
    loop {
        let accepted = tokio::select! {
            result = listener.accept() => result,
            _ = cancellation.recv() => break,
        };
        let Ok((mut socket, peer)) = accepted else {
            break;
        };
        let control = control.clone();
        let cancel = cancel.clone();
        let kind = info.kind;
        let target_address = info.target_address.clone();
        let target_port = info.target_port;
        tokio::spawn(async move {
            let (target_address, target_port) = if kind == SshForwardingType::Dynamic {
                match timeout(
                    Duration::from_secs(30),
                    forwarding::socks5_connect(&mut socket),
                )
                .await
                {
                    Ok(target) => {
                        let Ok(target) = target else {
                            let _ = forwarding::send_socks5_failure(&mut socket, 1).await;
                            return;
                        };
                        if forwarding::send_socks5_success(&mut socket).await.is_err() {
                            return;
                        }
                        target
                    }
                    Err(_) => {
                        let _ = forwarding::send_socks5_failure(&mut socket, 1).await;
                        return;
                    }
                }
            } else {
                (target_address, target_port)
            };
            let (sender, receiver) = oneshot::channel();
            if control
                .send(SshControl::OpenDirectTcpip {
                    host: target_address,
                    port: target_port,
                    sender,
                })
                .await
                .is_err()
            {
                return;
            }
            let Ok(Ok(channel)) = receiver.await else {
                return;
            };
            let mut ssh_stream = channel.into_stream();
            let mut cancellation = cancel.subscribe();
            let copy = async {
                let _ = peer;
                let _ = tokio::io::copy_bidirectional(&mut socket, &mut ssh_stream).await;
            };
            tokio::select! {
                _ = copy => {}
                _ = cancellation.recv() => {
                    let _ = ssh_stream.shutdown().await;
                    let _ = socket.shutdown().await;
                }
            }
        });
    }
    manager.finish_forwarding(&app, &info.id);
}

#[cfg(unix)]
enum X11Target {
    Unix(tokio::net::UnixStream),
    Tcp(TcpStream),
}

#[cfg(unix)]
async fn connect_x11_display(display: &str) -> Result<X11Target, std::io::Error> {
    let display = display.strip_prefix("unix:").unwrap_or(display);
    if display.starts_with('/') {
        return tokio::net::UnixStream::connect(display)
            .await
            .map(X11Target::Unix);
    }
    let (host, display_number) = if let Some(number) = display.strip_prefix(':') {
        ("unix", number)
    } else {
        display.rsplit_once(':').ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid DISPLAY")
        })?
    };
    let number = display_number
        .split('.')
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid DISPLAY"))?;
    if host == "unix" {
        return tokio::net::UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))
            .await
            .map(X11Target::Unix);
    }
    TcpStream::connect((host, number.saturating_add(6000)))
        .await
        .map(X11Target::Tcp)
}

fn emit_forwarding(app: &AppHandle, info: &SshForwardingInfo) {
    let _ = app.emit("ssh:forwardingChanged", info.clone());
}

fn x11_cookie(display: &str) -> String {
    #[cfg(unix)]
    if let Ok(output) = std::process::Command::new("xauth")
        .args(["nlist", display])
        .output()
    {
        if let Some(cookie) = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_whitespace().last())
            .find(|cookie| {
                cookie.len() == 32
                    && cookie
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
            })
        {
            return cookie.into();
        }
    }
    let mut random = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut random);
    random.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn emit_channel_message(
    app: &AppHandle,
    id: &str,
    connection_id: &str,
    profile_id: &str,
    message: ChannelMsg,
) -> bool {
    let event = match message {
        ChannelMsg::Data { data } => app.emit(
            "ssh:output",
            SshOutputEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                data: data.to_vec(),
                extended: false,
            },
        ),
        ChannelMsg::ExtendedData { data, .. } => app.emit(
            "ssh:output",
            SshOutputEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                data: data.to_vec(),
                extended: true,
            },
        ),
        ChannelMsg::ExitStatus { exit_status } => app.emit(
            "ssh:exit",
            SshExitEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                exit_code: Some(exit_status),
                signal: None,
            },
        ),
        ChannelMsg::ExitSignal { signal_name, .. } => app.emit(
            "ssh:exit",
            SshExitEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                exit_code: None,
                signal: Some(format!("{signal_name:?}")),
            },
        ),
        ChannelMsg::Eof | ChannelMsg::Close => {
            return true;
        }
        _ => return true,
    };
    event.is_ok()
}

#[derive(Debug, PartialEq, Eq)]
enum HostKeyDecisionAction {
    AcceptOnce,
    Save,
}

fn host_key_decision_action(
    status: HostKeyStatus,
    decision: HostKeyDecision,
) -> Result<HostKeyDecisionAction, SshError> {
    match (status, decision) {
        (HostKeyStatus::Unknown, HostKeyDecision::Once) => Ok(HostKeyDecisionAction::AcceptOnce),
        (HostKeyStatus::Unknown, HostKeyDecision::Save) => Ok(HostKeyDecisionAction::Save),
        (HostKeyStatus::Changed, _) => Err(SshError::HostKeyChanged),
        (HostKeyStatus::Unknown, HostKeyDecision::Reject) => Err(SshError::HostKeyRejected),
    }
}

#[cfg(unix)]
type PlatformAgentClient = AgentClient<tokio::net::UnixStream>;

#[cfg(windows)]
type PlatformAgentClient = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

fn authenticate_publickey_with_agent<'a>(
    handle: &'a mut client::Handle<SshHandler>,
    username: &str,
    identity: russh::keys::PublicKey,
    agent: &'a mut PlatformAgentClient,
) -> Pin<Box<dyn Future<Output = Result<AuthResult, russh::AgentAuthError>> + Send + 'a>> {
    Box::pin(handle.authenticate_publickey_with(username.to_owned(), identity, None, agent))
}

async fn wait_for_channel_confirmation(
    channel: &mut russh::Channel<client::Msg>,
) -> Result<Vec<ChannelMsg>, SshError> {
    let mut pending = Vec::new();
    loop {
        let message = tokio::time::timeout(CHANNEL_REQUEST_TIMEOUT, channel.wait())
            .await
            .map_err(|_| SshError::ChannelOpen)?
            .ok_or(SshError::ChannelOpen)?;
        match message {
            ChannelMsg::Success => return Ok(pending),
            ChannelMsg::Failure
            | ChannelMsg::Eof
            | ChannelMsg::Close
            | ChannelMsg::OpenFailure(_) => return Err(SshError::ChannelOpen),
            message => pending.push(message),
        }
    }
}

async fn connect_agent(socket: Option<String>) -> Result<PlatformAgentClient, SshError> {
    #[cfg(unix)]
    {
        let client = match socket {
            Some(path) => AgentClient::connect_uds(path).await,
            None => AgentClient::connect_env().await,
        }
        .map_err(|_| SshError::AuthenticationRejected)?;
        return Ok(client);
    }

    #[cfg(windows)]
    {
        let client = match socket {
            Some(path) => AgentClient::connect_named_pipe(path)
                .await
                .map(|client| client.dynamic()),
            None => match std::env::var_os("SSH_AUTH_SOCK") {
                Some(path) => AgentClient::connect_named_pipe(path)
                    .await
                    .map(|client| client.dynamic()),
                None => Ok(AgentClient::connect_pageant().await.dynamic()),
            },
        }
        .map_err(|_| SshError::AuthenticationRejected)?;
        return Ok(client);
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = socket;
        Err(SshError::InvalidRequest(
            "SSH agent authentication is unavailable on this platform".into(),
        ))
    }
}

fn validate_request(request: &SshConnectRequest) -> Result<(), SshError> {
    if request.jump_chain.len() > 3 {
        return Err(SshError::InvalidRequest(
            "at most three SSH jump hosts are supported".into(),
        ));
    }
    if request.connection_id.as_deref().is_some_and(|value| {
        value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
    }) {
        return Err(SshError::InvalidRequest(
            "SSH connection identifier is invalid".into(),
        ));
    }
    if request.profile_id.is_empty()
        || request.profile_id.len() > 256
        || request.profile_id.chars().any(char::is_control)
        || request.host.is_empty()
        || request.host.len() > 255
        || request
            .host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        || request.port == 0
    {
        return Err(SshError::InvalidRequest("SSH target is invalid".into()));
    }
    let username = request.username.as_deref().unwrap_or("root");
    if username.is_empty() || username.len() > 255 || username.chars().any(char::is_control) {
        return Err(SshError::InvalidRequest("SSH username is invalid".into()));
    }
    let terminal = &request.terminal;
    if terminal.term.is_empty()
        || terminal.term.len() > 64
        || terminal.term.chars().any(char::is_control)
        || terminal.columns == 0
        || terminal.rows == 0
        || terminal.columns > 1000
        || terminal.rows > 1000
    {
        return Err(SshError::InvalidRequest(
            "terminal request is invalid".into(),
        ));
    }
    if request.environment.len() > 64
        || request.environment.iter().any(|(name, value)| {
            name.is_empty()
                || name.len() > 128
                || value.len() > 8192
                || name.chars().any(|character| character.is_control())
                || value.chars().any(|character| character.is_control())
        })
    {
        return Err(SshError::InvalidRequest(
            "SSH environment is invalid".into(),
        ));
    }
    if let Some(KeepaliveOptions {
        interval_ms,
        max_count,
    }) = request.keepalive.as_ref()
    {
        if *interval_ms < 100 || *interval_ms > 86_400_000 || *max_count == 0 || *max_count > 100 {
            return Err(SshError::InvalidRequest(
                "SSH keepalive options are invalid".into(),
            ));
        }
    }
    for hop in &request.jump_chain {
        if hop.host.is_empty()
            || hop.host.len() > 255
            || hop.port == 0
            || hop
                .host
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(SshError::InvalidRequest("SSH jump host is invalid".into()));
        }
        if hop.username.as_deref().is_some_and(|value| {
            value.is_empty() || value.len() > 255 || value.chars().any(char::is_control)
        }) {
            return Err(SshError::InvalidRequest(
                "SSH jump host username is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn jump_request(
    root: &SshConnectRequest,
    hop: &SshJumpRequest,
    connection_id: &str,
    index: usize,
) -> SshConnectRequest {
    SshConnectRequest {
        profile_id: format!("{}#jump-{}", root.profile_id, index),
        connection_id: Some(connection_id.into()),
        host: hop.host.clone(),
        port: hop.port,
        username: hop.username.clone(),
        auth: hop.auth.clone(),
        terminal: root.terminal.clone(),
        keepalive: root.keepalive.clone(),
        environment: BTreeMap::new(),
        x11: false,
        x11_display: None,
        agent_forward: false,
        jump_chain: Vec::new(),
    }
}

fn validate_forwarding_request(request: &SshForwardingRequest) -> Result<(), SshError> {
    if request.session_id.is_empty()
        || request.session_id.len() > 256
        || request.session_id.chars().any(char::is_control)
    {
        return Err(SshError::InvalidRequest(
            "forwarding session identifier is invalid".into(),
        ));
    }
    forwarding::validate_bind_host(&request.bind_host)?;
    if request.kind != SshForwardingType::Dynamic {
        forwarding::validate_endpoint(
            &request.target_address,
            request.target_port,
            "forward target",
        )?;
    }
    Ok(())
}

fn resolve_secret_ref(
    reference: &str,
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Result<secrecy::SecretString, SshError> {
    if let Some(encoded) = reference.strip_prefix("vault-secret://") {
        let bytes = BASE64_STANDARD
            .decode(encoded)
            .map_err(|_| SshError::InvalidRequest("secret reference is invalid".into()))?;
        let selector: VaultSecretSelector = serde_json::from_slice(&bytes)
            .map_err(|_| SshError::InvalidRequest("secret reference is invalid".into()))?;
        let value = secrets
            .get_secret(&selector)
            .map_err(|_| SshError::AuthenticationRejected)?
            .ok_or(SshError::AuthenticationRejected)?;
        return Ok(secrecy::SecretString::new(value));
    }
    let Some(reference) = reference.strip_prefix("keychain://") else {
        return Err(SshError::InvalidRequest(
            "SSH secrets must use keychain:// or vault-secret:// references".into(),
        ));
    };
    let (service, account) = reference
        .split_once('/')
        .ok_or_else(|| SshError::InvalidRequest("secret reference is invalid".into()))?;
    let value = credentials
        .store()
        .get(
            CredentialNamespace::TabbyRs,
            &CredentialAddress {
                service: service.into(),
                account: account.into(),
            },
        )
        .map_err(|_| SshError::AuthenticationRejected)?
        .ok_or(SshError::AuthenticationRejected)?;
    Ok(value)
}

fn resolve_saved_private_key_passphrase(
    openssh: &[u8],
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Option<secrecy::SecretString> {
    let hash = hex::encode(Sha512::digest(openssh));
    let mut key = Map::new();
    key.insert("hash".into(), Value::String(hash.clone()));
    let selector = VaultSecretSelector {
        r#type: VAULT_SECRET_TYPE_PASSPHRASE.into(),
        key,
    };
    if let Ok(Some(value)) = secrets.get_secret(&selector) {
        return Some(secrecy::SecretString::new(value));
    }

    credentials
        .store()
        .get(
            CredentialNamespace::TabbyRs,
            &CredentialAddress {
                service: format!("ssh-private-key:{hash}"),
                account: "user".into(),
            },
        )
        .ok()
        .flatten()
}

impl From<SshError> for crate::error::AppError {
    fn from(error: SshError) -> Self {
        match error {
            SshError::InvalidRequest(message) => Self::InvalidArgument(message),
            SshError::HostKeyRejected | SshError::HostKeyChanged => {
                Self::PermissionDenied(error.to_string())
            }
            SshError::AuthenticationRejected | SshError::KeyParse => {
                Self::PermissionDenied(error.to_string())
            }
            SshError::Connection | SshError::ChannelOpen | SshError::Closed | SshError::Timeout => {
                Self::Io(error.to_string())
            }
            SshError::Internal => Self::Io("SSH operation failed".into()),
            SshError::Sftp(message) => Self::Io(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        engine::{
            KeyboardInteractiveResponse, PrivateKeyMaterial, SshAuthContext, SshAuthenticator,
        },
        host_key_decision_action,
        model::*,
        resolve_saved_private_key_passphrase, validate_request, HostKeyDecisionAction,
        ManagerAuthenticator, SshControl, SshManager, SshSession, VAULT_SECRET_TYPE_PASSPHRASE,
    };
    use crate::security::{
        CredentialAddress, CredentialError, CredentialNamespace, CredentialState, CredentialStore,
        SecretState, VaultSnapshot, VaultSnapshotSecret,
    };
    use async_trait::async_trait;
    use secrecy::{ExposeSecret, SecretString};
    use sha2::{Digest, Sha512};
    use std::{collections::BTreeMap, sync::Arc, time::Duration};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    #[derive(Default)]
    struct TestCredentialStore {
        value: Option<(String, String, String)>,
    }

    impl CredentialStore for TestCredentialStore {
        fn get(
            &self,
            namespace: CredentialNamespace,
            address: &CredentialAddress,
        ) -> Result<Option<SecretString>, CredentialError> {
            let Some((service, account, value)) = &self.value else {
                return Ok(None);
            };
            if namespace != CredentialNamespace::TabbyRs
                || service != &address.service
                || account != &address.account
            {
                return Ok(None);
            }
            Ok(Some(SecretString::new(value.clone())))
        }

        fn put(
            &self,
            _namespace: CredentialNamespace,
            _address: &CredentialAddress,
            _value: &str,
        ) -> Result<(), CredentialError> {
            Err(CredentialError::Unavailable)
        }

        fn delete(
            &self,
            _namespace: CredentialNamespace,
            _address: &CredentialAddress,
        ) -> Result<bool, CredentialError> {
            Err(CredentialError::Unavailable)
        }
    }

    struct RecordingAuthContext {
        calls: Vec<String>,
        password_valid: bool,
        private_key_passphrase: Option<String>,
    }

    #[async_trait]
    impl SshAuthContext for RecordingAuthContext {
        async fn authenticate_none(&mut self, _username: &str) -> Result<bool, SshError> {
            self.calls.push("none".into());
            Ok(true)
        }

        async fn authenticate_password(
            &mut self,
            _username: &str,
            password: &SecretString,
        ) -> Result<bool, SshError> {
            self.calls.push("password".into());
            self.password_valid = password.expose_secret() == "secret-password";
            Ok(false)
        }

        async fn authenticate_private_key(
            &mut self,
            _username: &str,
            key: PrivateKeyMaterial,
        ) -> Result<bool, SshError> {
            self.calls
                .push(format!("private-key:{}", key.openssh.len()));
            self.private_key_passphrase = key
                .passphrase
                .as_ref()
                .map(|value| value.expose_secret().to_owned());
            Ok(true)
        }

        async fn authenticate_agent(
            &mut self,
            _username: &str,
            socket: Option<&str>,
        ) -> Result<bool, SshError> {
            self.calls
                .push(format!("agent:{}", socket.unwrap_or("default")));
            Ok(true)
        }

        async fn authenticate_keyboard_interactive_start(
            &mut self,
            _username: &str,
        ) -> Result<KeyboardInteractiveResponse, SshError> {
            self.calls.push("keyboard-interactive".into());
            Ok(KeyboardInteractiveResponse::Success)
        }

        async fn authenticate_keyboard_interactive_respond(
            &mut self,
            _responses: Vec<String>,
        ) -> Result<KeyboardInteractiveResponse, SshError> {
            Ok(KeyboardInteractiveResponse::Success)
        }
    }

    fn request() -> SshConnectRequest {
        SshConnectRequest {
            profile_id: "ssh:test".into(),
            connection_id: Some("connection:test".into()),
            host: "example.test".into(),
            port: 22,
            username: Some("alice".into()),
            auth: vec![AuthMethodRef::KeyboardInteractive],
            terminal: TerminalRequest {
                term: "xterm-256color".into(),
                columns: 80,
                rows: 24,
                pixel_width: None,
                pixel_height: None,
            },
            keepalive: None,
            environment: BTreeMap::new(),
            x11: false,
            x11_display: None,
            agent_forward: false,
            jump_chain: Vec::new(),
        }
    }

    #[test]
    fn rejects_invalid_terminal_dimensions_and_control_data() {
        let mut value = request();
        value.terminal.columns = 0;
        assert!(validate_request(&value).is_err());
        let mut value = request();
        value.host = "example\n.test".into();
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn rejects_zero_keepalive_max_count() {
        let mut value = request();
        value.keepalive = Some(KeepaliveOptions {
            interval_ms: 5_000,
            max_count: 0,
        });
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn accepts_x11_and_rejects_excessive_jump_chain_before_connecting() {
        let mut value = request();
        value.x11 = true;
        assert!(validate_request(&value).is_ok());

        let mut value = request();
        value.jump_chain = (0..4)
            .map(|index| SshJumpRequest {
                host: format!("jump-{index}.example.test"),
                port: 22,
                username: Some("alice".into()),
                auth: Vec::new(),
            })
            .collect();
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn auth_method_accepts_only_secret_references() {
        let value: AuthMethodRef = serde_json::from_value(serde_json::json!({
            "type": "password",
            "secretRef": "keychain://ssh/example"
        }))
        .unwrap();
        match value {
            AuthMethodRef::Password { secret_ref } => {
                assert_eq!(secret_ref, "keychain://ssh/example");
            }
            _ => panic!("expected password secret reference"),
        }

        assert!(serde_json::from_value::<AuthMethodRef>(serde_json::json!({
            "type": "password",
            "password": "plaintext"
        }))
        .is_err());
    }

    #[test]
    fn changed_host_keys_cannot_be_saved_or_accepted_once() {
        assert!(matches!(
            host_key_decision_action(HostKeyStatus::Changed, HostKeyDecision::Save),
            Err(SshError::HostKeyChanged)
        ));
        assert!(matches!(
            host_key_decision_action(HostKeyStatus::Changed, HostKeyDecision::Once),
            Err(SshError::HostKeyChanged)
        ));
        assert!(matches!(
            host_key_decision_action(HostKeyStatus::Unknown, HostKeyDecision::Save),
            Ok(HostKeyDecisionAction::Save)
        ));
    }

    #[tokio::test]
    async fn manager_authenticator_runs_password_then_private_key_without_exposing_plaintext() {
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some(("ssh".into(), "alice".into(), "secret-password".into()));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let mut value = request();
        value.auth = vec![
            AuthMethodRef::Password {
                secret_ref: "keychain://ssh/alice".into(),
            },
            AuthMethodRef::PrivateKey {
                file_ref: private_key.to_string_lossy().into_owned(),
                passphrase_ref: None,
            },
        ];
        let authenticator = ManagerAuthenticator {
            manager: SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: value,
            secrets: &secrets,
            credentials: &credentials,
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            private_key_passphrase: None,
        };

        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert!(context.password_valid);
        assert_eq!(context.calls, ["password", "private-key:16"]);
    }

    #[tokio::test]
    async fn manager_authenticator_reuses_saved_private_key_passphrase() {
        let private_key_bytes = b"test-private-key";
        let hash = hex::encode(Sha512::digest(private_key_bytes));
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some((
            format!("ssh-private-key:{hash}"),
            "user".into(),
            "fixture-passphrase".into(),
        ));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, private_key_bytes).unwrap();
        let mut value = request();
        value.auth = vec![AuthMethodRef::PrivateKey {
            file_ref: private_key.to_string_lossy().into_owned(),
            passphrase_ref: None,
        }];
        let authenticator = ManagerAuthenticator {
            manager: SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: value,
            secrets: &secrets,
            credentials: &credentials,
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            private_key_passphrase: None,
        };

        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(
            context.private_key_passphrase.as_deref(),
            Some("fixture-passphrase")
        );
    }

    #[test]
    fn resolves_saved_private_key_passphrase_from_vault() {
        let private_key_bytes = b"test-private-key";
        let hash = hex::encode(Sha512::digest(private_key_bytes));
        let mut key = serde_json::Map::new();
        key.insert("hash".into(), serde_json::Value::String(hash));
        let secrets = SecretState::default();
        secrets
            .replace(
                VaultSnapshot {
                    config: serde_json::Value::Null,
                    secrets: vec![VaultSnapshotSecret {
                        r#type: VAULT_SECRET_TYPE_PASSPHRASE.into(),
                        key,
                        value: "fixture-passphrase".into(),
                    }],
                },
                SecretString::new("vault-passphrase".into()),
                Duration::from_secs(60),
            )
            .unwrap();

        let credentials = CredentialState::default();
        let passphrase =
            resolve_saved_private_key_passphrase(private_key_bytes, &secrets, &credentials)
                .expect("vault passphrase should resolve");
        assert_eq!(passphrase.expose_secret(), "fixture-passphrase");
    }

    #[tokio::test]
    async fn manager_authenticator_supports_agent_and_keyboard_interactive_paths() {
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let directory = tempdir().unwrap();
        let mut value = request();
        value.auth = vec![AuthMethodRef::Agent {
            socket: Some("/tmp/agent.sock".into()),
        }];
        let authenticator = ManagerAuthenticator {
            manager: SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: value,
            secrets: &secrets,
            credentials: &credentials,
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            private_key_passphrase: None,
        };
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls, ["agent:/tmp/agent.sock"]);

        let mut value = request();
        value.auth = vec![AuthMethodRef::KeyboardInteractive];
        let authenticator = ManagerAuthenticator {
            manager: SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: value,
            secrets: &secrets,
            credentials: &credentials,
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            private_key_passphrase: None,
        };
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls, ["keyboard-interactive"]);
    }

    #[tokio::test]
    async fn closing_one_session_does_not_affect_another_session() {
        let directory = tempdir().unwrap();
        let manager = SshManager::new(directory.path().join("known_hosts"));
        let (first_sender, mut first_receiver) = mpsc::channel(1);
        let (second_sender, mut second_receiver) = mpsc::channel(1);
        manager.sessions.lock().unwrap().insert(
            "first".into(),
            SshSession {
                control: first_sender,
            },
        );
        manager.sessions.lock().unwrap().insert(
            "second".into(),
            SshSession {
                control: second_sender,
            },
        );

        let first_task = tokio::spawn(async move {
            match first_receiver.recv().await {
                Some(SshControl::Close(sender)) => {
                    sender.send(Ok(())).unwrap();
                }
                Some(_) => panic!("first session received the wrong control message"),
                None => panic!("first session control channel closed before close"),
            }
        });
        let second_task = tokio::spawn(async move {
            match second_receiver.recv().await {
                Some(SshControl::Write(data, sender)) => {
                    assert_eq!(data, b"still-alive");
                    sender.send(Ok(())).unwrap();
                }
                Some(_) => panic!("second session received the wrong control message"),
                None => panic!("second session control channel closed unexpectedly"),
            }
        });

        manager
            .close(SshSessionIdRequest { id: "first".into() })
            .await
            .unwrap();
        manager.sessions.lock().unwrap().remove("first");
        manager
            .write(SshWriteRequest {
                id: "second".into(),
                data: b"still-alive".to_vec(),
            })
            .await
            .unwrap();

        first_task.await.unwrap();
        second_task.await.unwrap();
    }
}
