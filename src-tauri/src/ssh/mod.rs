pub mod agent;
pub mod engine;
#[cfg(test)]
mod engine_integration;
mod forwarding;
mod import;
mod known_hosts;
pub mod model;
mod pending;
pub mod sftp;

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fs,
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
    client::{self, Handler},
    keys::agent::client::AgentClient,
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
            HostKeyVerifier, KeyboardInteractiveResponse, PrivateKeyMaterial,
            RusshEngine, SshAuthContext, SshAuthenticator, SshHostKey, SshTarget,
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
    pending_connections: pending::PendingConnections,
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
    cancellation: pending::CancelSignal,
    host: String,
    port: u16,
    connection_id: String,
    app: AppHandle,
    host_key_error: Arc<Mutex<Option<SshError>>>,
    remote_routes: Arc<Mutex<HashMap<(String, String, u32), RemoteForwardRoute>>>,
    agent_forward: bool,
    agent_socket: Option<String>,
    x11: bool,
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
        match self.cancellation.run(verifier.verify(&self.host, self.port, &key)).await {
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
        tokio::spawn(forward_agent_channel(channel, socket, self.agent_forward));
        async { Ok(()) }
    }

    fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let app = self.app.clone();
        let connection_id = self.connection_id.clone();
        tokio::spawn(forward_x11_channel(
            channel,
            self.x11_display.clone(),
            self.x11,
            move |message| {
                let _ = app.emit(
                    "ssh:message",
                    serde_json::json!({
                        "connectionId": connection_id, "message": message,
                    }),
                );
            },
        ));
        async { Ok(()) }
    }
}

async fn forward_x11_channel(
    channel: russh::Channel<client::Msg>,
    display: Option<String>,
    enabled: bool,
    on_message: impl Fn(String) + Send,
) -> Result<(), russh::Error> {
    if !enabled {
        let _ = channel.close().await;
        return Ok(());
    }
    let display = x11_display_spec(display, std::env::var_os("DISPLAY"));
    match connect_x11_display(&display).await {
        #[cfg(unix)]
        Ok(X11Target::Unix(mut target)) => {
            let mut ssh_stream = channel.into_stream();
            let _ = copy_bidirectional(&mut target, &mut ssh_stream).await;
        }
        Ok(X11Target::Tcp(mut target)) => {
            let mut ssh_stream = channel.into_stream();
            let _ = copy_bidirectional(&mut target, &mut ssh_stream).await;
        }
        Err(error) => {
            for message in x11_failure_messages(&display, &error, cfg!(windows)) {
                on_message(message);
            }
            let _ = channel.close().await;
        }
    }
    Ok(())
}

fn x11_failure_messages(display: &str, error: &std::io::Error, windows: bool) -> Vec<String> {
    let terminal_text = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_control() {
                    c.escape_default().to_string()
                } else {
                    c.to_string()
                }
            })
            .collect()
    };
    let endpoint = parse_x11_display(display, windows).to_json();
    let mut messages = vec![
        format!(
            "\x1b[41m\x1b[30m X \x1b[39m\x1b[49m Could not connect to the X server: {}",
            terminal_text(&error.to_string())
        ),
        format!(
            "    Tabby RS tried to connect to {} based on the DISPLAY environment var ({})",
            terminal_text(&endpoint),
            terminal_text(display)
        ),
    ];
    if windows {
        messages.extend(
            [
                "    To use X forwarding, you need a local X server, e.g.:",
                "    * VcXsrv: https://sourceforge.net/projects/vcxsrv/",
                "    * Xming: https://sourceforge.net/projects/xming/",
            ]
            .map(str::to_owned),
        );
    }
    messages
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

struct ManagerAuthenticator<'a> {
    manager: SshManager,
    app: Option<AppHandle>,
    request: SshConnectRequest,
    secrets: &'a SecretState,
    credentials: &'a CredentialState,
    used_private_key: Mutex<bool>,
    resolved_username: Mutex<Option<String>>,
}

impl ManagerAuthenticator<'_> {
    fn username(&self) -> Result<String, SshError> {
        self.resolved_username.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone().ok_or(SshError::AuthenticationRejected)
    }

    fn record_private_key(&self, used: bool) {
        *self
            .used_private_key
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = used;
    }

    fn used_private_key(&self) -> bool {
        *self
            .used_private_key
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait::async_trait]
impl SshAuthenticator for ManagerAuthenticator<'_> {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        _username: &str,
        methods: &[AuthMethodRef],
    ) -> Result<bool, SshError> {
        self.record_private_key(false);
        *self.resolved_username.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let resolved_username = resolve_username_with(
            self.request.username.as_deref(),
            || async {
                let app = self.app.as_ref().ok_or(SshError::Closed)?;
                self.manager.prompt_for_responses(app, SshAuthPrompt {
                    request_id: self.manager.new_id("auth"),
                    id: self.request.profile_id.clone(),
                    connection_id: self.request.connection_id.clone().unwrap_or_else(|| self.request.profile_id.clone()),
                    name: format!("Username for {}", self.request.host),
                    instructions: String::new(),
                    prompts: vec![SshAuthPromptItem { text: "Username".into(), echo: true }],
                    username: true,
                    keyboard_interactive: None,
                    saved_password: None,
                    password: None,
                    private_key_hash: None,
                }).await
            },
            |name| std::env::var(name).ok(),
        ).await?;
        *self.resolved_username.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(resolved_username.clone());
        let username = resolved_username.as_str();
        let methods = if methods.is_empty() {
            &self.request.auth
        } else {
            methods
        };
        let passwords = resolve_auth_passwords(methods, &self.request, username, self.secrets, self.credentials)?;
        if context.authenticate_none(username).await? { return Ok(true); }
        let mut remaining: Vec<usize> = (0..methods.len()).collect();
        while let Some(position) = remaining.iter().position(|&index| {
            let allowed = context.remaining_auth_methods();
            allowed.is_empty() || allowed.iter().any(|name| name == auth_method_name(&methods[index]))
        }) {
            let index = remaining.remove(position);
            let method = &methods[index];
            let result = match method {
                AuthMethodRef::ProvidedPassword { password } => {
                    context.authenticate_password(username, password).await
                }
                AuthMethodRef::Password { .. } => {
                    match passwords.values[index].as_ref() {
                        Some(password) => {
                            let already_tried = methods[..index].iter().any(|method| {
                                matches!(method, AuthMethodRef::ProvidedPassword { password: provided }
                                    if provided.expose_secret() == password.expose_secret())
                            });
                            if already_tried {
                                continue;
                            }
                            context.authenticate_password(username, password).await
                        }
                        None => Ok(false),
                    }
                }
                AuthMethodRef::PromptPassword => {
                    let app = self.app.as_ref().ok_or(SshError::Closed)?;
                    let request_id = self.manager.new_id("auth");
                    let connection_id = self
                        .request
                        .connection_id
                        .clone()
                        .unwrap_or_else(|| self.request.profile_id.clone());
                    let responses = self
                        .manager
                        .prompt_for_responses(
                            app,
                            SshAuthPrompt {
                                request_id: request_id.clone(),
                                id: self.request.profile_id.clone(),
                                connection_id: connection_id.clone(),
                                name: format!("Password for {username}@{}", self.request.host),
                                instructions: String::new(),
                                prompts: vec![SshAuthPromptItem {
                                    text: "Password".into(),
                                    echo: false,
                                }],
                                password: Some(SshPasswordPromptTarget {
                                    host: self.request.host.clone(),
                                    port: self.request.port,
                                    username: username.into(),
                                }),
                                username: false,
                                keyboard_interactive: None,
                                saved_password: None,
                                private_key_hash: None,
                            },
                        )
                        .await?;
                    let authenticated =
                        authenticate_password_response(context, username, responses).await?;
                    if authenticated {
                        app.emit(
                            "ssh:passwordAccepted",
                            SshCredentialAccepted {
                                request_id,
                                connection_id,
                            },
                        )
                        .map_err(|_| SshError::Closed)?;
                    }
                    Ok(authenticated)
                }
                AuthMethodRef::PrivateKey {
                    file_ref,
                    passphrase_ref,
                } => {
                    let mut material = match self
                        .manager
                        .load_private_key_material(
                            file_ref,
                            passphrase_ref.as_deref(),
                            self.secrets,
                            self.credentials,
                        )
                        .await
                    {
                        Ok(material) => material,
                        Err(SshError::KeyParse | SshError::AuthenticationRejected) => continue,
                        Err(error) => return Err(error),
                    };
                    let mut prompted_request_id = None;
                    let result = loop {
                        match context
                            .authenticate_private_key(username, material.clone())
                            .await
                        {
                            Err(SshError::KeyPassphrase) if passphrase_ref.is_none() => {
                                let request_id = self.manager.new_id("auth");
                                let responses = self
                                    .manager
                                    .prompt_for_responses(
                                        self.app.as_ref().ok_or(SshError::Closed)?,
                                        SshAuthPrompt {
                                            request_id: request_id.clone(),
                                            id: self.request.profile_id.clone(),
                                            connection_id: self
                                                .request
                                                .connection_id
                                                .clone()
                                                .unwrap_or_else(|| self.request.profile_id.clone()),
                                            name: "Private key passphrase".into(),
                                            password: None,
                                            username: false,
                                            keyboard_interactive: None,
                                            saved_password: None,
                                            private_key_hash: Some(hex::encode(Sha512::digest(
                                                &material.openssh,
                                            ))),
                                            instructions: "The private key is encrypted.".into(),
                                            prompts: vec![SshAuthPromptItem {
                                                text: "Passphrase".into(),
                                                echo: false,
                                            }],
                                        },
                                    )
                                    .await?;
                                let mut responses = zeroize::Zeroizing::new(responses);
                                if responses.is_empty() {
                                    break Ok(false);
                                }
                                if responses.len() != 1 {
                                    break Err(SshError::InvalidRequest(
                                        "one passphrase response is required".into(),
                                    ));
                                }
                                material.passphrase =
                                    Some(secrecy::SecretString::new(responses[0].clone()));
                                responses.zeroize();
                                prompted_request_id = Some(request_id);
                            }
                            Ok(authenticated) => {
                                if let Some(request_id) = prompted_request_id {
                                    self.app
                                        .as_ref()
                                        .ok_or(SshError::Closed)?
                                        .emit(
                                            "ssh:privateKeyUnlocked",
                                            SshCredentialAccepted {
                                                request_id,
                                                connection_id: self
                                                    .request
                                                    .connection_id
                                                    .clone()
                                                    .unwrap_or_else(|| {
                                                        self.request.profile_id.clone()
                                                    }),
                                            },
                                        )
                                        .map_err(|_| SshError::Closed)?;
                                }
                                break Ok(authenticated);
                            }
                            Err(error) => break Err(error),
                        }
                    };
                    let result = match result {
                        Err(SshError::KeyParse | SshError::KeyPassphrase) => Ok(false),
                        result => result,
                    };
                    if let Ok(completed) = &result {
                        if *completed || context.private_key_was_accepted() {
                            self.record_private_key(true);
                        }
                    }
                    result
                }
                AuthMethodRef::Agent { socket } => {
                    match context
                        .authenticate_agent(username, socket.as_deref())
                        .await
                    {
                        Err(SshError::AuthenticationRejected) => Ok(false),
                        result => result,
                    }
                }
                AuthMethodRef::KeyboardInteractive { .. } => {
                    let mut response = context
                        .authenticate_keyboard_interactive_start(username)
                        .await?;
                    loop {
                        match response {
                            KeyboardInteractiveResponse::Success => break Ok(true),
                            KeyboardInteractiveResponse::Failure => break Ok(false),
                            KeyboardInteractiveResponse::Prompt(prompt) => {
                                if prompt.prompts.is_empty() {
                                    response = context
                                        .authenticate_keyboard_interactive_respond(Vec::new())
                                        .await?;
                                    continue;
                                }
                                let mut responses = self
                                    .manager
                                    .prompt_for_responses(
                                        self.app.as_ref().ok_or(SshError::Closed)?,
                                        keyboard_interactive_prompt(&self.request, username, prompt, passwords.values[index].clone()),
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
        // An unreadable store may contain a password that never became a candidate.
        if passwords.storage_unavailable {
            return Err(SshError::AuthenticationRejected);
        }
        // Only the exhausted-candidate path identifies a password for deletion.
        Err(SshError::AuthenticationExhausted(SshPasswordPromptTarget {
            host: self.request.host.clone(),
            port: self.request.port,
            username: username.into(),
        }))
    }
}

async fn resolve_username_with<F, Fut, E>(
    configured: Option<&str>,
    prompt: F,
    environment: E,
) -> Result<String, SshError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, SshError>>,
    E: FnOnce(&str) -> Option<String>,
{
    let username = match configured.filter(|value| !value.is_empty()) {
        Some(username) => username.to_owned(),
        None => {
            let mut responses = zeroize::Zeroizing::new(prompt().await?);
            if responses.is_empty() {
                return Err(SshError::Closed);
            }
            if responses.len() != 1 {
                return Err(SshError::InvalidRequest("one username response is required".into()));
            }
            responses.remove(0)
        }
    };
    let username = username.strip_prefix('$').and_then(environment).unwrap_or(username);
    if username.is_empty() {
        return Err(SshError::AuthenticationRejected);
    }
    if username.len() > 255 || username.chars().any(char::is_control) {
        return Err(SshError::InvalidRequest("SSH username is invalid".into()));
    }
    Ok(username)
}

async fn authenticate_password_response(
    context: &mut dyn SshAuthContext,
    username: &str,
    responses: Vec<String>,
) -> Result<bool, SshError> {
    let mut responses = zeroize::Zeroizing::new(responses);
    if responses.is_empty() {
        return Ok(false);
    }
    if responses.len() != 1 {
        return Err(SshError::InvalidRequest(
            "one password response is required".into(),
        ));
    }
    let password = secrecy::SecretString::new(responses[0].clone());
    responses.zeroize();
    context.authenticate_password(username, &password).await
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
            pending_connections: pending::PendingConnections::default(),
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
            cancellation: self.pending_connections.signal(connection_id).unwrap_or_default(),
            host,
            port,
            connection_id: connection_id.into(),
            app: app.clone(),
            host_key_error,
            remote_routes: Arc::clone(&self.remote_routes),
            agent_forward: request.agent_forward,
            agent_socket: request.forwarding_agent_socket(),
            x11: request.x11,
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
            async {
                let stream = tokio::net::TcpStream::connect((request.host.as_str(), request.port)).await?;
                if config.nodelay {
                    if let Err(error) = stream.set_nodelay(true) {
                        eprintln!("SSH TCP_NODELAY failed: {error}");
                    }
                }
                client::connect_stream(config, handler.cancellation.wrap(stream), handler).await
            },
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
    ) -> Result<(client::Handle<SshHandler>, bool, String), SshError> {
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
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        });
        let engine = RusshEngine::from_shared_config(config, Duration::from_secs(30))
            .with_cancellation(handler.cancellation.clone());
        let handle = engine
            .connect_with_handler(
                &SshTarget {
                    host: request.host.clone(),
                    port: request.port,
                    username: request.username.clone().unwrap_or_default(),
                },
                handler,
                host_key_error,
                authenticator.as_ref(),
            )
            .await?;
        Ok((handle, authenticator.used_private_key(), authenticator.username()?))
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
            client::connect_stream(config, handler.cancellation.wrap(channel.into_stream()), handler),
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
        let connection_id = request
            .connection_id
            .clone()
            .unwrap_or_else(|| request.profile_id.clone());
        let pending = self.pending_connections.begin(&connection_id)?;
        // The acknowledgement lets a renderer that closed before registration cancel again.
        app.emit("ssh:connecting", SshConnectionIdRequest { connection_id: connection_id.clone() })
            .map_err(|_| SshError::Closed)?;
        let result = pending.signal.run(self.connect_inner(app, request, secrets, credentials, connection_id)).await;
        if result.is_err() {
            pending.signal.cancel();
        }
        result
    }

    pub fn cancel_connect(&self, connection_id: &str) {
        self.pending_connections.cancel(connection_id);
    }

    async fn connect_inner(
        &self,
        app: AppHandle,
        request: SshConnectRequest,
        secrets: Arc<SecretState>,
        credentials: CredentialState,
        connection_id: String,
    ) -> Result<SshSessionInfo, SshError> {
        let username;
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
        let used_private_key;
        if request.jump_chain.is_empty() {
            let (connected, connected_with_private_key, connected_username) = self
                .connect_direct_engine(
                    Arc::clone(&config),
                    &app,
                    &request,
                    &connection_id,
                    &secrets,
                    &credentials,
                )
                .await?;
            handle = connected;
            used_private_key = connected_with_private_key;
            username = connected_username;
        } else {
            let first = jump_request(&request, &request.jump_chain[0], &connection_id, 0);
            handle = self
                .connect_direct(Arc::clone(&config), &app, &first, &connection_id)
                .await?;
            let first_authenticated = match self
                .authenticate(
                    &app,
                    &mut handle,
                    &first,
                    &secrets,
                    &credentials,
                )
                .await
            {
                Ok((authenticated, _, _)) => authenticated,
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
                let next_authenticated = match self
                    .authenticate(
                        &app,
                        &mut handle,
                        &next,
                        &secrets,
                        &credentials,
                    )
                    .await
                {
                    Ok((authenticated, _, _)) => authenticated,
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
                    &secrets,
                    &credentials,
                )
                .await
            {
                Ok((authenticated, target_used_private_key, target_username)) => {
                    used_private_key = target_used_private_key;
                    username = target_username;
                    authenticated
                }
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
            let display = x11_display_spec(request.x11_display.clone(), std::env::var_os("DISPLAY"));
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
            used_private_key,
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

    pub async fn auth_response(&self, mut request: SshAuthResponseRequest) -> Result<(), SshError> {
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
        if request.abort {
            request.responses.zeroize();
            drop(sender);
            return Ok(());
        }
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
        let _reply = pending::ReplyGuard::insert(&self.host_key_waiters, request_id.clone(), sender);
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
            return Err(SshError::HostKeyRejected);
        }
        let decision = timeout(HOST_KEY_TIMEOUT, receiver).await;
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
        secrets: &SecretState,
        credentials: &CredentialState,
    ) -> Result<(bool, bool, String), SshError> {
        let authenticator = ManagerAuthenticator {
            manager: self.clone(),
            app: Some(app.clone()),
            request: request.clone(),
            secrets,
            credentials,
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        };
        let mut context = engine::RusshAuthContext::new(handle);
        let authenticated = authenticator
            .authenticate(&mut context, request.username.as_deref().unwrap_or_default(), &request.auth)
            .await?;
        Ok((authenticated, authenticator.used_private_key(), authenticator.username()?))
    }

    async fn prompt_for_responses(
        &self,
        app: &AppHandle,
        mut prompt: SshAuthPrompt,
    ) -> Result<Vec<String>, SshError> {
        let request_id = if prompt.request_id.is_empty() {
            self.new_id("auth")
        } else {
            prompt.request_id.clone()
        };
        prompt.request_id = request_id.clone();
        let (sender, receiver) = oneshot::channel();
        let _reply = pending::ReplyGuard::insert(&self.auth_waiters, request_id, sender);
        if app.emit("ssh:authPrompt", prompt).is_err() {
            return Err(SshError::Closed);
        }
        let result = timeout(AUTH_PROMPT_TIMEOUT, receiver).await;
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

enum X11Target {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    Tcp(TcpStream),
}

#[derive(Debug, PartialEq)]
enum X11Address {
    Unix(String),
    Tcp(String, f64),
}

impl X11Address {
    fn to_json(&self) -> String {
        match self {
            Self::Unix(path) => serde_json::json!({ "path": path }).to_string(),
            Self::Tcp(host, port) => format!(
                "{{\"host\":{},\"port\":{}}}",
                serde_json::to_string(host).unwrap(),
                if port.is_finite() {
                    x11_number(*port)
                } else {
                    "null".into()
                },
            ),
        }
    }
}

fn x11_number(number: f64) -> String {
    if number.is_infinite() {
        "Infinity".into()
    } else if number >= 1e21 {
        format!("{number:e}").replace('e', "e+")
    } else {
        number.to_string()
    }
}

fn x11_display_spec(configured: Option<String>, environment: Option<std::ffi::OsString>) -> String {
    configured
        .filter(|value| !value.is_empty())
        .or_else(|| environment.map(|value| value.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "localhost:0".into())
}

fn parse_x11_display(display: &str, windows: bool) -> X11Address {
    if display.starts_with('/') {
        return X11Address::Unix(display.into());
    }
    // Preserve 14e2d60's greedy JS regex, including its wildcard separator and
    // default on a missing match. JS's non-Unicode dot matches one UTF-16 unit.
    static SPEC: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let spec = SPEC.get_or_init(|| regex::Regex::new(
        r"\A([^\n\r\u{2028}\u{2029}]+):([0-9]+)[^\n\r\u{2028}\u{2029}\u{10000}-\u{10ffff}]([0-9]+)\z",
    ).expect("fixed upstream DISPLAY regex"));
    let captures = spec.captures(display);
    let host = captures
        .as_ref()
        .map_or(if windows { "localhost" } else { "unix" }, |c| {
            c.get(1).unwrap().as_str()
        });
    let number = captures
        .as_ref()
        .map_or(0.0, |c| c[2].parse::<f64>().expect("ASCII decimal digits"));
    if host == "unix" {
        return X11Address::Unix(format!("/tmp/.X11-unix/X{}", x11_number(number)));
    }
    // Tabby treats numbers below 100 as display indices and larger values as TCP ports.
    let port = if number < 100.0 {
        number + 6000.0
    } else {
        number
    };
    X11Address::Tcp(host.into(), port)
}

async fn connect_x11_display(display: &str) -> Result<X11Target, std::io::Error> {
    match parse_x11_display(display, cfg!(windows)) {
        #[cfg(unix)]
        X11Address::Unix(path) => tokio::net::UnixStream::connect(path)
            .await
            .map(X11Target::Unix),
        #[cfg(not(unix))]
        X11Address::Unix(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "X11 Unix sockets are not supported on this platform",
        )),
        X11Address::Tcp(host, port) => {
            if !port.is_finite() || !(0.0..=65535.0).contains(&port) || port.fract() != 0.0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid DISPLAY port: {}", x11_number(port)),
                ));
            }
            TcpStream::connect((host.as_str(), port as u16))
                .await
                .map(X11Target::Tcp)
        }
    }
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

async fn forward_agent_channel(
    channel: russh::Channel<client::Msg>,
    socket: Option<String>,
    enabled: bool,
) -> Result<(), russh::Error> {
    if !enabled {
        let _ = channel.close().await;
        return Ok(());
    }
    let Ok(mut agent) = connect_agent(socket)
        .await
        .map(|client| client.into_inner())
    else {
        let _ = channel.close().await;
        return Ok(());
    };
    let mut ssh_stream = channel.into_stream();
    let _ = copy_bidirectional(&mut agent, &mut ssh_stream).await;
    Ok(())
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
            None => Ok(AgentClient::connect_pageant().await.dynamic()),
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

fn keyboard_interactive_prompt(
    request: &SshConnectRequest,
    username: &str,
    prompt: engine::KeyboardInteractivePrompt,
    saved_password: Option<secrecy::SecretString>,
) -> SshAuthPrompt {
    SshAuthPrompt {
        request_id: String::new(),
        id: request.profile_id.clone(),
        connection_id: request.connection_id.clone().unwrap_or_else(|| request.profile_id.clone()),
        name: prompt.name,
        instructions: prompt.instructions,
        prompts: prompt.prompts.into_iter().map(|item| SshAuthPromptItem { text: item.text, echo: item.echo }).collect(),
        username: false,
        password: None,
        private_key_hash: None,
        keyboard_interactive: Some(SshPasswordPromptTarget {
            host: request.host.clone(),
            port: request.port,
            username: username.into(),
        }),
        saved_password,
    }
}

fn validate_request(request: &SshConnectRequest) -> Result<(), SshError> {
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
        agent_forwarding: None,
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

fn auth_method_name(method: &AuthMethodRef) -> &'static str {
    match method {
        AuthMethodRef::ProvidedPassword { .. } | AuthMethodRef::Password { .. } | AuthMethodRef::PromptPassword => "password",
        AuthMethodRef::PrivateKey { .. } | AuthMethodRef::Agent { .. } => "publickey",
        AuthMethodRef::KeyboardInteractive { .. } => "keyboard-interactive",
    }
}

#[derive(Debug)]
struct ResolvedAuthPasswords {
    values: Vec<Option<secrecy::SecretString>>,
    storage_unavailable: bool,
}

fn resolve_auth_passwords(
    methods: &[AuthMethodRef],
    request: &SshConnectRequest,
    username: &str,
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Result<ResolvedAuthPasswords, SshError> {
    // Snapshot stored credentials before interactive panels can save new values.
    let mut saved = std::collections::HashMap::<String, Option<secrecy::SecretString>>::new();
    let mut storage_unavailable = false;
    let values = methods.iter().map(|method| {
        let reference = match method {
            AuthMethodRef::ProvidedPassword { password } => return Ok(Some(password.clone())),
            AuthMethodRef::KeyboardInteractive { password: Some(password), .. } => return Ok(Some(password.clone())),
            AuthMethodRef::KeyboardInteractive { secret_ref: Some(reference), .. }
            | AuthMethodRef::Password { secret_ref: reference } => reference,
            _ => return Ok(None),
        };
        if !saved.contains_key(reference) {
            let value = match resolve_password_ref(reference, request, username, secrets, credentials) {
                Ok(value) => value.filter(|value| !value.expose_secret().is_empty()),
                Err(SshError::AuthenticationRejected) => {
                    storage_unavailable = true;
                    None
                }
                Err(error) => return Err(error),
            };
            saved.insert(reference.clone(), value);
        }
        let value = saved.get(reference).cloned().flatten();
        if matches!(method, AuthMethodRef::KeyboardInteractive { .. }) && value.as_ref().is_some_and(|value| {
            methods.iter().any(|method| matches!(method,
                AuthMethodRef::KeyboardInteractive { password: Some(configured), .. }
                if configured.expose_secret() == value.expose_secret()))
        }) {
            // Upstream keeps the second bare candidate when configured and saved match.
            return Ok(None);
        }
        Ok(value)
    }).collect::<Result<Vec<_>, SshError>>()?;
    Ok(ResolvedAuthPasswords { values, storage_unavailable })
}

fn resolve_password_ref(
    reference: &str,
    request: &SshConnectRequest,
    username: &str,
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Result<Option<secrecy::SecretString>, SshError> {
    let resolved = match reference {
        "ssh-password://keychain" => format!("keychain://ssh@{}:{}/{username}", request.host, request.port),
        "ssh-password://vault" => {
            let selector = serde_json::json!({
                "type": "password",
                "key": { "user": username, "host": request.host, "port": request.port },
            });
            format!("vault-secret://{}", BASE64_STANDARD.encode(selector.to_string()))
        }
        _ => return lookup_secret_ref(reference, secrets, credentials),
    };
    lookup_secret_ref(&resolved, secrets, credentials)
}

fn resolve_secret_ref(
    reference: &str,
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Result<secrecy::SecretString, SshError> {
    lookup_secret_ref(reference, secrets, credentials)?.ok_or(SshError::AuthenticationRejected)
}

fn lookup_secret_ref(
    reference: &str,
    secrets: &SecretState,
    credentials: &CredentialState,
) -> Result<Option<secrecy::SecretString>, SshError> {
    if let Some(encoded) = reference.strip_prefix("vault-secret://") {
        let bytes = BASE64_STANDARD
            .decode(encoded)
            .map_err(|_| SshError::InvalidRequest("secret reference is invalid".into()))?;
        let selector: VaultSecretSelector = serde_json::from_slice(&bytes)
            .map_err(|_| SshError::InvalidRequest("secret reference is invalid".into()))?;
        let value = secrets
            .get_secret(&selector)
            .map_err(|_| SshError::AuthenticationRejected)?;
        return Ok(value.map(secrecy::SecretString::new));
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
        .map_err(|_| SshError::AuthenticationRejected)?;
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
            SshError::AuthenticationRejected | SshError::AuthenticationExhausted(_) | SshError::KeyParse | SshError::KeyPassphrase => {
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
            public_key_accepted, KeyboardInteractiveResponse, PrivateKeyMaterial, SshAuthContext,
            SshAuthenticator,
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
    use russh::client::AuthResult;
    use secrecy::{ExposeSecret, SecretString};
    use sha2::{Digest, Sha512};
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    #[cfg(unix)]
    #[test]
    fn x11_display_preserves_non_unicode_environment() {
        use std::os::unix::ffi::OsStringExt;
        let value = std::ffi::OsString::from_vec(b"/tmp/\xff:1.0".to_vec());
        assert_eq!(
            super::x11_display_spec(None, Some(value)),
            "/tmp/\u{fffd}:1.0"
        );
    }

    #[test]
    fn x11_display_selection_matches_upstream() {
        assert_eq!(
            super::x11_display_spec(Some("host:1.0".into()), Some("host:2.0".into())),
            "host:1.0"
        );
        assert_eq!(
            super::x11_display_spec(Some(String::new()), Some("host:2.0".into())),
            "host:2.0"
        );
        assert_eq!(
            super::x11_display_spec(None, Some(String::new().into())),
            ""
        );
        assert_eq!(super::x11_display_spec(None, None), "localhost:0");
        assert_eq!(
            super::x11_display_spec(Some(String::new()), None),
            "localhost:0"
        );
    }

    #[test]
    fn x11_display_matches_fixed_upstream_oracle() {
        let oracle: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/x11-display.json")).unwrap();
        for case in oracle["cases"].as_array().unwrap() {
            let display = case["display"].as_str().unwrap();
            let windows = case["windows"].as_bool().unwrap();
            let actual: serde_json::Value =
                serde_json::from_str(&super::parse_x11_display(display, windows).to_json())
                    .unwrap();
            assert_eq!(
                actual, case["expected"],
                "DISPLAY={display:?}, Windows={windows}"
            );
        }
    }

    #[test]
    fn x11_failure_reports_display_and_platform_guidance() {
        let error =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let messages = super::x11_failure_messages("localhost:100.0", &error, false);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].contains("Could not connect to the X server: connection refused"));
        assert_eq!(messages[1], "    Tabby RS tried to connect to {\"host\":\"localhost\",\"port\":100} based on the DISPLAY environment var (localhost:100.0)");
        let messages = super::x11_failure_messages("unix:0.0", &error, true);
        assert_eq!(messages.len(), 5);
        assert!(messages[1].contains("{\"path\":\"/tmp/.X11-unix/X0\"}"));
        assert_eq!(
            messages[2],
            "    To use X forwarding, you need a local X server, e.g.:"
        );
        assert_eq!(
            messages[3],
            "    * VcXsrv: https://sourceforge.net/projects/vcxsrv/"
        );
        assert_eq!(
            messages[4],
            "    * Xming: https://sourceforge.net/projects/xming/"
        );
        assert!(
            super::x11_failure_messages("invalid", &error, false)[1].contains("/tmp/.X11-unix/X0")
        );
    }

    #[test]
    fn x11_failure_escapes_terminal_controls() {
        let error = std::io::Error::other("bad\x1b]52;c;secret\x07\nerror");
        let messages = super::x11_failure_messages("/tmp/\x1b[2J\r\u{009b}", &error, false);
        assert!(!messages[0].contains("\x1b]52"));
        assert!(!messages[0].contains(['\x07', '\n']));
        assert!(!messages[1].chars().any(char::is_control));
        assert!(messages[0].contains("secret"));
    }

    #[test]
    fn x11_display_windows_and_unix_defaults_match_upstream() {
        use super::{parse_x11_display, X11Address};
        assert_eq!(
            parse_x11_display(":0", true),
            X11Address::Tcp("localhost".into(), 6000.0)
        );
        assert_eq!(
            parse_x11_display(":0", false),
            X11Address::Unix("/tmp/.X11-unix/X0".into())
        );
        for windows in [true, false] {
            assert_eq!(
                parse_x11_display("127.0.0.1:100.0", windows),
                X11Address::Tcp("127.0.0.1".into(), 100.0)
            );
            assert_eq!(
                parse_x11_display("unix:0.0", windows),
                X11Address::Unix("/tmp/.X11-unix/X0".into())
            );
        }
    }

    #[tokio::test]
    async fn x11_display_rejects_out_of_range_tcp_ports() {
        for display in ["host:65536.0", "host:1000000000000000000000.0"] {
            let error = match super::connect_x11_display(display).await {
                Err(error) => error,
                Ok(_) => panic!("invalid port must not connect"),
            };
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        }
    }

    #[tokio::test]
    async fn x11_display_connects_to_explicit_tcp_port() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(Duration::from_secs(3), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            assert!(port >= 100);
            let target = super::connect_x11_display(&format!("127.0.0.1:{port}.0"))
                .await
                .expect("X11 must use ports >=100 without adding 6000");
            let mut client = match target {
                super::X11Target::Tcp(client) => client,
                #[cfg(unix)]
                super::X11Target::Unix(_) => panic!("expected TCP"),
            };
            let (mut server, _) = listener.accept().await.unwrap();
            client.write_all(b"ping").await.unwrap();
            let mut bytes = [0; 4];
            server.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"ping");
            server.write_all(b"pong").await.unwrap();
            client.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"pong");
        })
        .await
        .expect("X11 TCP exchange timed out");
    }

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
        password_accepted: bool,
        private_key_passphrase: Option<String>,
        private_key_result: bool,
        private_key_accepted: bool,
        agent_error: Option<fn() -> SshError>,
    }

    #[async_trait]
    impl SshAuthContext for RecordingAuthContext {
        async fn authenticate_none(&mut self, _username: &str) -> Result<bool, SshError> {
            self.calls.push("none".into());
            Ok(false)
        }

        async fn authenticate_password(
            &mut self,
            _username: &str,
            password: &SecretString,
        ) -> Result<bool, SshError> {
            self.calls.push("password".into());
            self.password_valid = password.expose_secret() == "secret-password";
            Ok(self.password_accepted && self.password_valid)
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
            Ok(self.private_key_result)
        }

        fn private_key_was_accepted(&self) -> bool {
            self.private_key_accepted
        }

        async fn authenticate_agent(
            &mut self,
            _username: &str,
            socket: Option<&str>,
        ) -> Result<bool, SshError> {
            self.calls
                .push(format!("agent:{}", socket.unwrap_or("default")));
            match self.agent_error {
                Some(error) => Err(error()),
                None => Ok(true),
            }
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
            auth: vec![AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }],
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
            agent_forwarding: None,
            jump_chain: Vec::new(),
        }
    }

    fn request_with_agent_auth(socket: Option<&str>) -> SshConnectRequest {
        let mut value = request();
        value.auth = vec![
            AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
            AuthMethodRef::Agent {
                socket: socket.map(Into::into),
            },
        ];
        value
    }

    #[test]
    fn resolved_username_password_uses_selected_store_and_actual_target() {
        let mut request = request();
        request.username = Some("root".into());
        request.port = 2222;
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
            value: Some(("ssh@example.test:2222".into(), "bob".into(), "keychain-password".into())),
        }));
        let secrets = SecretState::default();
        let key = serde_json::json!({ "user": "bob", "host": "example.test", "port": 2222 });
        secrets.replace(VaultSnapshot {
            config: serde_json::Value::Null,
            secrets: vec![VaultSnapshotSecret {
                r#type: "password".into(), key: key.as_object().unwrap().clone(), value: "vault-password".into(),
            }],
        }, SecretString::new("fixture".into()), Duration::from_secs(60)).unwrap();
        for (reference, expected) in [
            ("ssh-password://keychain", "keychain-password"),
            ("ssh-password://vault", "vault-password"),
            ("keychain://ssh@example.test:2222/bob", "keychain-password"),
        ] {
            let value = super::resolve_password_ref(reference, &request, "bob", &secrets, &credentials).unwrap().unwrap();
            assert_eq!(value.expose_secret(), expected);
        }
        for (username, host, port) in [("root", "example.test", 2222), ("bob", "other.test", 2222), ("bob", "example.test", 22)] {
            request.host = host.into(); request.port = port;
            for reference in ["ssh-password://keychain", "ssh-password://vault"] {
                assert!(super::resolve_password_ref(reference, &request, username, &secrets, &credentials).unwrap().is_none());
            }
        }
        request.host = "example.test".into(); request.port = 2222;
        let empty_credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        assert!(super::resolve_password_ref("ssh-password://keychain", &request, "bob", &secrets, &empty_credentials).unwrap().is_none());
        assert!(super::resolve_password_ref("ssh-password://vault", &request, "bob", &SecretState::default(), &credentials).is_err());
    }

    #[tokio::test]
    async fn resolved_username_prompts_only_when_empty_and_expands_environment() {
        for (configured, response, expected, prompts) in [
            (Some("alice"), "ignored", "alice", 0),
            (None, "bob", "bob", 1),
            (Some(""), "bob", "bob", 1),
            (Some("$LOGIN"), "ignored", "resolved", 0),
            (None, "$LOGIN", "resolved", 1),
            (Some("$UNSET"), "ignored", "$UNSET", 0),
        ] {
            let called = std::cell::Cell::new(0);
            let result = super::resolve_username_with(configured, || {
                called.set(called.get() + 1);
                std::future::ready(Ok(vec![response.into()]))
            }, |name| (name == "LOGIN").then(|| "resolved".into())).await.unwrap();
            assert_eq!(result, expected); assert_eq!(called.get(), prompts);
        }
        for responses in [vec![], vec![String::new()], vec!["a".into(), "b".into()], vec!["bad\nuser".into()], vec!["a".repeat(256)]] {
            assert!(super::resolve_username_with(None, || std::future::ready(Ok(responses)), |_| None).await.is_err());
        }
        for resolved in [String::new(), "bad\nuser".into(), "a".repeat(256)] {
            assert!(super::resolve_username_with(Some("$LOGIN"), || async { panic!("must not prompt") }, |_| Some(resolved)).await.is_err());
        }
        assert!(matches!(super::resolve_username_with(None, || async { Err(SshError::Closed) }, |_| None).await, Err(SshError::Closed)));
    }

    #[test]
    fn forwarding_agent_explicit_path_overrides_auth() {
        let mut value = request_with_agent_auth(Some("/auth.sock"));
        value.agent_forwarding = Some(AgentForwardingOptions {
            socket: Some("/forward.sock".into()),
        });
        assert_eq!(
            value.forwarding_agent_socket().as_deref(),
            Some("/forward.sock")
        );

        let mut value = request();
        value.agent_forwarding = Some(AgentForwardingOptions {
            socket: Some("/forward.sock".into()),
        });
        assert_eq!(
            value.forwarding_agent_socket().as_deref(),
            Some("/forward.sock")
        );
    }

    #[test]
    fn forwarding_agent_explicit_null_overrides_auth_path() {
        let mut value = request_with_agent_auth(Some("/auth.sock"));
        value.agent_forwarding = Some(AgentForwardingOptions { socket: None });
        assert_eq!(value.forwarding_agent_socket(), None);
    }

    #[test]
    fn forwarding_agent_legacy_request_derives_from_auth() {
        let value = request_with_agent_auth(Some("/auth.sock"));
        assert_eq!(value.forwarding_agent_socket().as_deref(), Some("/auth.sock"));
        assert_eq!(request().forwarding_agent_socket(), None);
    }

    #[test]
    fn forwarding_agent_wire_field_is_optional() {
        let base = serde_json::json!({
            "profileId": "ssh:test",
            "host": "example.test",
            "port": 22,
            "username": "alice",
            "auth": [{ "type": "agent", "socket": "/auth.sock" }],
            "terminal": { "term": "xterm-256color", "columns": 80, "rows": 24, "pixelWidth": null, "pixelHeight": null },
            "keepalive": null,
            "agentForward": true
        });
        let omitted: SshConnectRequest = serde_json::from_value(base.clone()).unwrap();
        assert!(omitted.agent_forwarding.is_none());
        assert_eq!(omitted.forwarding_agent_socket().as_deref(), Some("/auth.sock"));

        let mut null_field = base.clone();
        null_field["agentForwarding"] = serde_json::Value::Null;
        let null_field: SshConnectRequest = serde_json::from_value(null_field).unwrap();
        assert!(null_field.agent_forwarding.is_none());

        let mut explicit_null = base.clone();
        explicit_null["agentForwarding"] = serde_json::json!({ "socket": null });
        let explicit_null: SshConnectRequest = serde_json::from_value(explicit_null).unwrap();
        assert_eq!(explicit_null.forwarding_agent_socket(), None);

        let mut explicit_path = base;
        explicit_path["agentForwarding"] = serde_json::json!({ "socket": "/forward.sock" });
        let explicit_path: SshConnectRequest = serde_json::from_value(explicit_path).unwrap();
        assert_eq!(
            explicit_path.forwarding_agent_socket().as_deref(),
            Some("/forward.sock")
        );
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
    fn accepts_x11_and_valid_long_jump_chains() {
        let mut value = request();
        value.x11 = true;
        assert!(validate_request(&value).is_ok());

        let mut value = request();
        value.jump_chain = (0..8)
            .map(|index| SshJumpRequest {
                host: format!("jump-{index}.example.test"),
                port: 22,
                username: Some("alice".into()),
                auth: Vec::new(),
            })
            .collect();
        assert!(validate_request(&value).is_ok());
        value.jump_chain[7].host = "bad host".into();
        assert!(validate_request(&value).is_err(), "every hop must still be validated");
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
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            password_accepted: false,
            private_key_passphrase: None,
            private_key_result: true,
            private_key_accepted: false,
            agent_error: None,
        };

        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert!(context.password_valid);
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
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
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            password_accepted: false,
            private_key_passphrase: None,
            private_key_result: true,
            private_key_accepted: false,
            agent_error: None,
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
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            password_accepted: false,
            private_key_passphrase: None,
            private_key_result: true,
            private_key_accepted: false,
            agent_error: None,
        };
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["agent:/tmp/agent.sock"]);

        let mut value = request();
        value.auth = vec![AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }];
        let authenticator = ManagerAuthenticator {
            manager: SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: value,
            secrets: &secrets,
            credentials: &credentials,
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        };
        let mut context = RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            password_accepted: false,
            private_key_passphrase: None,
            private_key_result: true,
            private_key_accepted: false,
            agent_error: None,
        };
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["keyboard-interactive"]);
    }

    fn recording_context(private_key_result: bool) -> RecordingAuthContext {
        RecordingAuthContext {
            calls: Vec::new(),
            password_valid: false,
            password_accepted: false,
            private_key_passphrase: None,
            private_key_result,
            private_key_accepted: false,
            agent_error: None,
        }
    }

    #[tokio::test]
    async fn keyboard_interactive_empty_challenges_continue_without_ui() {
        use super::engine::{KeyboardInteractivePrompt, KeyboardInteractivePromptItem};
        use std::collections::VecDeque;

        struct ChallengeContext {
            states: VecDeque<Result<KeyboardInteractiveResponse, SshError>>,
            answers: Vec<Vec<String>>,
            password_attempts: usize,
        }

        #[async_trait]
        impl SshAuthContext for ChallengeContext {
            async fn authenticate_none(&mut self, _: &str) -> Result<bool, SshError> {
                Ok(false)
            }
            async fn authenticate_password(&mut self, user: &str, password: &SecretString) -> Result<bool, SshError> {
                assert_eq!(user, "alice");
                assert_eq!(password.expose_secret(), "fallback");
                self.password_attempts += 1;
                Ok(true)
            }
            async fn authenticate_private_key(&mut self, _: &str, _: PrivateKeyMaterial) -> Result<bool, SshError> {
                panic!("unexpected private-key authentication")
            }
            async fn authenticate_agent(&mut self, _: &str, _: Option<&str>) -> Result<bool, SshError> {
                panic!("unexpected agent authentication")
            }
            async fn authenticate_keyboard_interactive_start(&mut self, user: &str) -> Result<KeyboardInteractiveResponse, SshError> {
                assert_eq!(user, "alice");
                self.states.pop_front().expect("start state")
            }
            async fn authenticate_keyboard_interactive_respond(&mut self, responses: Vec<String>) -> Result<KeyboardInteractiveResponse, SshError> {
                self.answers.push(responses);
                self.states.pop_front().expect("response state")
            }
        }

        let empty = || Ok(KeyboardInteractiveResponse::Prompt(KeyboardInteractivePrompt {
            name: "Server notice".into(), instructions: "No input is required".into(), prompts: vec![],
        }));
        let interactive = || Ok(KeyboardInteractiveResponse::Prompt(KeyboardInteractivePrompt {
            name: "Second factor".into(), instructions: String::new(),
            prompts: vec![KeyboardInteractivePromptItem { text: "Code".into(), echo: false }],
        }));
        let cases = [
            (vec![empty(), Ok(KeyboardInteractiveResponse::Success)], false, "accepted", 1, 0),
            (vec![empty(), empty(), Ok(KeyboardInteractiveResponse::Failure)], false, "rejected", 2, 0),
            (vec![empty(), empty(), Ok(KeyboardInteractiveResponse::Failure)], true, "accepted", 2, 1),
            (vec![empty(), interactive()], true, "closed", 1, 0),
            (vec![empty(), Err(SshError::Timeout)], true, "timeout", 1, 0),
            (vec![Err(SshError::Connection)], true, "connection", 0, 0),
        ];
        let directory = tempdir().unwrap();
        let secrets = SecretState::default();
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        for (states, fallback, expected, answers, password_attempts) in cases {
            let mut methods = vec![AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }];
            if fallback {
                methods.push(AuthMethodRef::ProvidedPassword { password: SecretString::new("fallback".into()) });
            }
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, methods);
            let mut context = ChallengeContext { states: states.into(), answers: vec![], password_attempts: 0 };
            let outcome = authenticator.authenticate(&mut context, "alice", &[]).await;
            let actual = match outcome { Ok(true) => "accepted", Err(SshError::AuthenticationExhausted(_)) => "rejected", Err(ref error) => error.code(), Ok(false) => panic!("missing rejected identity") };
            assert_eq!(actual, expected);
            assert_eq!(context.answers, vec![Vec::<String>::new(); answers]);
            assert_eq!(context.password_attempts, password_attempts);
            assert!(context.states.is_empty());
        }
        for accepted in [false, true] {
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
                AuthMethodRef::KeyboardInteractive { password: Some(SecretString::new("configured".into())), secret_ref: None },
                AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
                AuthMethodRef::ProvidedPassword { password: SecretString::new("fallback".into()) },
            ]);
            let mut context = ChallengeContext {
                states: vec![Ok(KeyboardInteractiveResponse::Failure), empty(),
                    Ok(if accepted { KeyboardInteractiveResponse::Success } else { KeyboardInteractiveResponse::Failure })].into(),
                answers: vec![], password_attempts: 0,
            };
            assert!(authenticator.authenticate(&mut context, "alice", &[]).await.unwrap());
            assert!(context.states.is_empty(), "Both interactive candidates must run before password fallback");
            assert_eq!(context.answers, vec![Vec::<String>::new()]);
            assert_eq!(context.password_attempts, usize::from(!accepted));
        }
    }

    #[test]
    fn keyboard_interactive_prompt_carries_resolved_connection_identity() {
        let mut target = request();
        target.username = Some("$USER".into());
        target.host = "jump.test".into();
        target.port = 2222;
        target.connection_id = Some("connection-1".into());
        let prompt = super::keyboard_interactive_prompt(&target, "resolved-user", super::engine::KeyboardInteractivePrompt {
            name: "Challenge".into(), instructions: "Instructions".into(),
            prompts: vec![super::engine::KeyboardInteractivePromptItem { text: "Password: ".into(), echo: false }],
        }, Some(SecretString::new("configured-secret".into())));
        assert!(!format!("{prompt:?}").contains("configured-secret"));
        let value = serde_json::to_value(prompt).unwrap();
        assert_eq!(value["savedPassword"], "configured-secret");
        assert_eq!(value["keyboardInteractive"], serde_json::json!({ "host": "jump.test", "port": 2222, "username": "resolved-user" }));
        assert_eq!(value["connectionId"], "connection-1");
        assert_eq!(value["name"], "Challenge");
        assert_eq!(value["instructions"], "Instructions");
        assert_eq!(value["prompts"], serde_json::json!([{ "text": "Password: ", "echo": false }]));
        assert!(value.get("password").is_none());
        assert!(value.get("privateKeyHash").is_none());
        assert_eq!(value["username"], false);
    }

    fn authenticator_for<'a>(
        known_hosts: &std::path::Path,
        secrets: &'a SecretState,
        credentials: &'a CredentialState,
        auth: Vec<AuthMethodRef>,
    ) -> ManagerAuthenticator<'a> {
        let mut value = request();
        value.auth = auth;
        ManagerAuthenticator {
            manager: SshManager::new(known_hosts.to_path_buf()),
            app: None,
            request: value,
            secrets,
            credentials,
            used_private_key: Mutex::new(false),
            resolved_username: Mutex::new(None),
        }
    }

    fn used_private_key(authenticator: &ManagerAuthenticator<'_>) -> bool {
        *authenticator.used_private_key.lock().unwrap()
    }

    fn authentication_succeeded(result: Result<bool, SshError>) -> bool {
        match result {
            Ok(true) => true,
            Err(SshError::AuthenticationExhausted(target)) => {
                assert_eq!((target.host.as_str(), target.port, target.username.as_str()),
                    ("example.test", 22, "alice"));
                false
            }
            other => panic!("unexpected authentication result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn exhausted_authentication_carries_the_resolved_account() {
        let directory = tempdir().unwrap();
        let secrets = SecretState::default();
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        for (host, port, username) in [("target.test", 22, "target-user"), ("hop.test", 2222, "hop-user")] {
            let mut authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
                AuthMethodRef::ProvidedPassword { password: SecretString::new("wrong-password".into()) },
            ]);
            authenticator.request.host = host.into();
            authenticator.request.port = port;
            authenticator.request.username = Some(username.into());
            let mut context = recording_context(false);
            let result = authenticator.authenticate(&mut context, "wrong-default-user", &[]).await;
            match result {
                Err(SshError::AuthenticationExhausted(target)) => {
                    assert_eq!(target.host, host);
                    assert_eq!(target.port, port);
                    assert_eq!(target.username, username);
                }
                other => panic!("expected account-specific exhaustion, got {other:?}"),
            }
            assert_eq!(context.calls, ["none", "password"]);
        }
    }

    #[tokio::test]
    async fn inaccessible_password_storage_never_authorizes_deletion() {
        struct Unavailable;
        impl CredentialStore for Unavailable {
            fn get(&self, _: CredentialNamespace, _: &CredentialAddress) -> Result<Option<SecretString>, CredentialError> {
                Err(CredentialError::Unavailable)
            }
            fn put(&self, _: CredentialNamespace, _: &CredentialAddress, _: &str) -> Result<(), CredentialError> {
                unreachable!()
            }
            fn delete(&self, _: CredentialNamespace, _: &CredentialAddress) -> Result<bool, CredentialError> {
                unreachable!()
            }
        }
        let directory = tempdir().unwrap();
        let secrets = SecretState::default(); // Locked, not an empty unlocked Vault.
        let credentials = CredentialState::with_store(Arc::new(Unavailable));
        for reference in ["ssh-password://vault", "ssh-password://keychain"] {
            for accepted in [false, true] {
                let authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
                    AuthMethodRef::ProvidedPassword { password: SecretString::new("secret-password".into()) },
                    AuthMethodRef::Password { secret_ref: reference.into() },
                ]);
                let mut context = recording_context(false);
                context.password_accepted = accepted;
                let result = authenticator.authenticate(&mut context, "alice", &[]).await;
                if accepted { assert!(matches!(result, Ok(true))); }
                else { assert!(matches!(result, Err(SshError::AuthenticationRejected)), "{result:?}"); }
                assert_eq!(context.calls, ["none", "password"]);
            }
        }
    }

    #[tokio::test]
    async fn resolved_username_manager_uses_request_identity_before_password_lookup() {
        let secrets = SecretState::default();
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
            value: Some(("ssh@example.test:22".into(), "bob".into(), "secret-password".into())),
        }));
        let directory = tempdir().unwrap();
        let mut authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
            AuthMethodRef::Password { secret_ref: "ssh-password://keychain".into() },
        ]);
        authenticator.request.username = Some("bob".into());
        let mut context = recording_context(false);
        context.password_accepted = true;
        assert!(authenticator.authenticate(&mut context, "root", &[]).await.unwrap());
        assert_eq!(authenticator.username().unwrap(), "bob");
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["password"]);
        authenticator.request.username = None;
        let mut context = recording_context(false);
        assert!(matches!(authenticator.authenticate(&mut context, "root", &[]).await, Err(SshError::Closed)));
        assert!(context.calls.is_empty(), "no default-root attempt when a username cannot be supplied");
        assert!(authenticator.username().is_err(), "a cancelled retry must clear the previous identity");
    }

    #[tokio::test]
    async fn auth_response_distinguishes_prompt_dismissal_from_connection_abort() {
        let directory = tempdir().unwrap();
        let manager = SshManager::new(directory.path().join("known_hosts"));
        for (abort, responses) in [(None, vec![]), (Some(false), vec![""]), (Some(true), vec![])] {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            manager.auth_waiters.lock().unwrap().insert("auth-test".into(), sender);
            let mut value = serde_json::json!({ "requestId": "auth-test", "responses": responses });
            if let Some(abort) = abort { value["abort"] = serde_json::json!(abort); }
            let request = serde_json::from_value(value).unwrap();
            manager.auth_response(request).await.unwrap();
            if abort == Some(true) {
                assert!(receiver.await.is_err(), "aborting must close the waiter rather than skip a candidate");
            } else {
                assert_eq!(receiver.await.unwrap(), responses);
            }
            assert!(manager.auth_waiters.lock().unwrap().is_empty());
            let duplicate = serde_json::from_value(serde_json::json!({
                "requestId": "auth-test", "responses": [], "abort": true,
            })).unwrap();
            assert!(matches!(manager.auth_response(duplicate).await, Err(SshError::InvalidRequest(_))));
        }
    }

    #[tokio::test]
    async fn password_response_preserves_cancellation_and_password_results() {
        let mut context = recording_context(false);
        assert!(matches!(
            super::authenticate_password_response(&mut context, "alice", vec![]).await,
            Ok(false)
        ));
        assert!(context.calls.is_empty(), "dismissing the prompt skips this candidate without sending an empty password");
        assert!(matches!(
            super::authenticate_password_response(&mut context, "alice", vec!["a".into(), "b".into()]).await,
            Err(SshError::InvalidRequest(_))
        ));
        assert!(context.calls.is_empty());
        for (password, accepted, expected) in [
            ("secret-password", true, true),
            ("wrong-password", true, false),
            ("secret-password", false, false),
            ("", true, false),
        ] {
            let mut context = recording_context(false);
            context.password_accepted = accepted;
            assert_eq!(
                super::authenticate_password_response(&mut context, "alice", vec![password.into()]).await.unwrap(),
                expected
            );
            assert_eq!(context.calls, ["password"]);
        }
    }

    #[tokio::test]
    async fn provided_password_authenticates_and_falls_back_without_exposing_secrets() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
            value: Some(("ssh".into(), "alice".into(), "secret-password".into())),
        }));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        for (password, fallback, accepted, attempts) in [
            ("secret-password", false, true, 1),
            ("wrong-password", false, false, 1),
            ("wrong-password", true, true, 2),
            ("secret-password", true, true, 1),
        ] {
            let method: AuthMethodRef = serde_json::from_value(serde_json::json!({
                "type": "providedPassword", "password": password,
            })).expect("decode configured password");
            let mut methods = vec![method];
            if fallback {
                methods.push(AuthMethodRef::Password { secret_ref: "keychain://ssh/alice".into() });
            }
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, methods);
            let debug = format!("{:?}", authenticator.request);
            assert!(!debug.contains(password), "request Debug must redact configured passwords");
            let mut context = recording_context(false);
            context.password_accepted = true;
            assert_eq!(authentication_succeeded(authenticator.authenticate(&mut context, "alice", &[]).await), accepted);
            assert_eq!(context.password_valid, accepted);
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(context.calls, vec!["password"; attempts]);
            assert!(!used_private_key(&authenticator));
        }
    }

    #[tokio::test]
    async fn provided_password_is_not_retried_from_the_credential_store() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
            value: Some(("ssh".into(), "alice".into(), "expired-password".into())),
        }));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        for fallback in [false, true] {
            let mut methods = vec![
                AuthMethodRef::ProvidedPassword { password: SecretString::new("expired-password".into()) },
                AuthMethodRef::Password { secret_ref: "keychain://ssh/alice".into() },
            ];
            if fallback { methods.push(AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }); }
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, methods);
            let mut context = recording_context(false);
            context.password_accepted = true;
            assert_eq!(authentication_succeeded(authenticator.authenticate(&mut context, "alice", &[]).await), fallback);
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(context.calls, if fallback { vec!["password", "keyboard-interactive"] } else { vec!["password"] });
        }
    }

    #[test]
    fn password_response_wire_contract() {
        let method: AuthMethodRef = serde_json::from_value(serde_json::json!({ "type": "promptPassword" })).unwrap();
        assert!(matches!(method, AuthMethodRef::PromptPassword));
        let accepted = super::SshCredentialAccepted {
            request_id: "prompt:1".into(),
            connection_id: "connection:1".into(),
        };
        assert_eq!(serde_json::to_value(accepted).unwrap(), serde_json::json!({
            "requestId": "prompt:1", "connectionId": "connection:1"
        }));
    }

    #[test]
    fn keyboard_interactive_candidates_snapshot_saved_passwords() {
        let directory = tempdir().unwrap();
        let secrets = SecretState::default();
        for stored in [None, Some("configured"), Some("stored")] {
            let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
                value: stored.map(|value| ("ssh@example.test:22".into(), "alice".into(), value.into())),
            }));
            let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
                { "type": "providedPassword", "password": "configured" },
                { "type": "keyboardInteractive", "password": "configured" },
                { "type": "keyboardInteractive", "secretRef": "ssh-password://keychain" },
                { "type": "password", "secretRef": "ssh-password://keychain" },
                { "type": "promptPassword" }
            ])).unwrap();
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, methods);
            let prepared = super::resolve_auth_passwords(&authenticator.request.auth, &authenticator.request, "alice", &secrets, &credentials).unwrap();
            let values: Vec<_> = prepared.values.iter().map(|value| value.as_ref().map(|value| value.expose_secret().as_str())).collect();
            assert_eq!(values, vec![Some("configured"), Some("configured"), stored.filter(|value| *value != "configured"), stored, None]);
            assert!(!format!("{prepared:?}").contains("configured"));
            let isolated = super::resolve_auth_passwords(&authenticator.request.auth, &authenticator.request, "other-user", &secrets, &credentials).unwrap();
            assert!(isolated.values[2].is_none());
            assert!(isolated.values[3].is_none());
        }
    }

    #[test]
    fn keyboard_interactive_vault_snapshot_survives_panel_password_saving() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        let secrets = SecretState::default();
        let snapshot = |value: &str| VaultSnapshot {
            config: serde_json::Value::Null,
            secrets: vec![VaultSnapshotSecret { r#type: "password".into(),
                key: serde_json::json!({"user":"alice","host":"example.test","port":22}).as_object().unwrap().clone(),
                value: value.into() }],
        };
        secrets.replace(snapshot("initial-secret"), SecretString::new("fixture".into()), Duration::from_secs(60)).unwrap();
        let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
            {"type":"keyboardInteractive","secretRef":"ssh-password://vault"},
            {"type":"password","secretRef":"ssh-password://vault"}
        ])).unwrap();
        let prepared = super::resolve_auth_passwords(&methods, &request(), "alice", &secrets, &credentials).unwrap();
        secrets.replace(snapshot("new-secret"), SecretString::new("fixture".into()), Duration::from_secs(60)).unwrap();
        for password in prepared.values { assert_eq!(password.unwrap().expose_secret(), "initial-secret"); }
        for (host, port, username) in [("other.test", 22, "alice"), ("example.test", 23, "alice"), ("example.test", 22, "bob")] {
            let mut target = request(); target.host = host.into(); target.port = port;
            let prepared = super::resolve_auth_passwords(&methods, &target, username, &secrets, &credentials).unwrap();
            assert!(prepared.values.iter().all(Option::is_none));
        }
        let legacy: AuthMethodRef = serde_json::from_value(serde_json::json!({"type":"keyboardInteractive"})).unwrap();
        assert!(matches!(legacy, AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }));
    }

    #[tokio::test]
    async fn manager_authenticator_follows_server_method_changes_over_transport() {
        use russh::{server, MethodKind, MethodSet};
        struct Client;
        impl russh::client::Handler for Client {
            type Error = russh::Error;
            async fn check_server_key(&mut self, _: &russh::keys::PublicKey) -> Result<bool, Self::Error> { Ok(true) }
        }
        struct Server { calls: Arc<Mutex<Vec<&'static str>>>, mode: &'static str }
        fn reject(method: MethodKind) -> server::Auth {
            server::Auth::Reject { proceed_with_methods: Some(MethodSet::from(&[method][..])), partial_success: false }
        }
        impl server::Handler for Server {
            type Error = russh::Error;
            async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
                self.calls.lock().unwrap().push("none");
                Ok(if self.mode == "none accepted" { server::Auth::Accept } else { reject(MethodKind::Password) })
            }
            async fn auth_password(&mut self, _: &str, password: &str) -> Result<server::Auth, Self::Error> {
                self.calls.lock().unwrap().push("password");
                Ok(if password == "secret-password" { server::Auth::Accept }
                    else if self.mode == "methods change" { reject(MethodKind::KeyboardInteractive) }
                    else if self.mode == "empty update" { server::Auth::Reject { proceed_with_methods: Some(MethodSet::empty()), partial_success: false } }
                    else { reject(MethodKind::Password) })
            }
            async fn auth_keyboard_interactive<'a>(&'a mut self, _: &str, _: &str, _: Option<server::Response<'a>>) -> Result<server::Auth, Self::Error> {
                self.calls.lock().unwrap().push("keyboard-interactive"); Ok(reject(MethodKind::Password))
            }
        }
        for mode in ["password only", "methods change", "empty update", "none accepted"] {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let mut config = server::Config::default();
            config.auth_rejection_time = Duration::ZERO;
            config.auth_rejection_time_initial = Some(Duration::ZERO);
            config.keys.push(russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519).unwrap());
            let calls = Arc::new(Mutex::new(Vec::new()));
            let observed = calls.clone();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let result = server::run_stream(Arc::new(config), socket, Server { calls, mode }).await.unwrap().await;
                if mode == "empty update" {
                    assert!(result.is_ok() || matches!(result, Err(russh::Error::IO(ref error)) if error.kind() == std::io::ErrorKind::UnexpectedEof));
                } else { result.unwrap(); }
            });
            let mut handle = russh::client::connect(Arc::new(russh::client::Config::default()), address, Client).await.unwrap();
            let directory = tempdir().unwrap();
            let secrets = SecretState::default();
            let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
                AuthMethodRef::KeyboardInteractive { password: Some(SecretString::new("configured".into())), secret_ref: None },
                AuthMethodRef::ProvidedPassword { password: SecretString::new("wrong".into()) },
                AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
                AuthMethodRef::ProvidedPassword { password: SecretString::new("secret-password".into()) },
            ]);
            let mut context = super::engine::RusshAuthContext::new(&mut handle);
            let result = tokio::time::timeout(Duration::from_secs(5), authenticator.authenticate(&mut context, "alice", &[])).await.unwrap();
            if mode == "empty update" {
                // russh closes the transport on an empty server method list.
                assert!(matches!(result, Err(SshError::AuthenticationRejected)));
                assert_eq!(context.remaining_auth_methods(), ["password"]);
            } else {
                assert!(matches!(result, Ok(true)), "{mode}: {result:?}; observed {:?}", observed.lock().unwrap());
            }
            assert_eq!(*observed.lock().unwrap(), if mode == "empty update" { vec!["none", "password"] }
            else if mode == "none accepted" { vec!["none"] } else if mode == "methods change" {
                vec!["none", "password", "keyboard-interactive", "password"]
            } else { vec!["none", "password", "password"] });
            let disconnect = handle.disconnect(russh::Disconnect::ByApplication, "fixture done", "").await;
            if mode != "empty update" { disconnect.unwrap(); }
            tokio::time::timeout(Duration::from_secs(5), server).await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn manager_authenticator_skips_missing_saved_password() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let password = AuthMethodRef::Password {
            secret_ref: "keychain://ssh/alice".into(),
        };
        for fallback in [false, true] {
            let mut methods = vec![password.clone()];
            if fallback {
                methods.push(AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None });
            }
            let authenticator =
                authenticator_for(directory.path(), &secrets, &credentials, methods);
            let mut context = recording_context(false);
            assert_eq!(
                authentication_succeeded(authenticator
                    .authenticate(&mut context, "alice", &[])
                    .await),
                fallback
            );
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(
                context.calls,
                if fallback {
                    vec!["keyboard-interactive"]
                } else {
                    vec![]
                }
            );
        }
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![
                AuthMethodRef::Password {
                    secret_ref: "invalid-reference".into(),
                },
                AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
            ],
        );
        let mut context = recording_context(false);
        assert!(matches!(
            authenticator.authenticate(&mut context, "alice", &[]).await,
            Err(SshError::InvalidRequest(_))
        ));
        assert!(context.calls.is_empty());
    }

    fn private_key_method(file_ref: &std::path::Path) -> AuthMethodRef {
        AuthMethodRef::PrivateKey {
            file_ref: file_ref.to_string_lossy().into_owned(),
            passphrase_ref: None,
        }
    }

    #[tokio::test]
    async fn manager_authenticator_skips_unavailable_private_key_material() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore {
            value: Some(("ssh".into(), "alice".into(), "secret-password".into())),
        }));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let valid_key = directory.path().join("valid-key");
        std::fs::write(&valid_key, b"test-private-key").unwrap();
        let missing_key = private_key_method(&directory.path().join("missing-key"));
        let password = AuthMethodRef::Password {
            secret_ref: "keychain://ssh/alice".into(),
        };
        let unavailable = [
            missing_key,
            private_key_method(directory.path()),
            AuthMethodRef::PrivateKey {
                file_ref: "vault://missing-key".into(),
                passphrase_ref: None,
            },
            AuthMethodRef::PrivateKey {
                file_ref: valid_key.to_string_lossy().into_owned(),
                passphrase_ref: Some("keychain://missing/passphrase".into()),
            },
        ];
        for method in unavailable {
            for fallback in [None, Some(password.clone()), Some(private_key_method(&valid_key))] {
                let expects_key = matches!(&fallback, Some(AuthMethodRef::PrivateKey { .. }));
                let expected = fallback.is_some();
                let mut methods = vec![method.clone()];
                methods.extend(fallback);
                let authenticator = authenticator_for(directory.path(), &secrets, &credentials, methods);
                let mut context = recording_context(true);
                context.password_accepted = true;
                assert_eq!(
                    authentication_succeeded(authenticator.authenticate(&mut context, "alice", &[]).await),
                    expected,
                );
                assert_eq!(used_private_key(&authenticator), expects_key);
                let expected_calls = if expects_key {
                    vec!["private-key:16"]
                } else if expected {
                    vec!["password"]
                } else {
                    vec![]
                };
                assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
                assert_eq!(context.calls, expected_calls);
            }
        }
    }

    #[test]
    fn private_key_decode_distinguishes_corruption_from_passphrase_errors() {
        use russh::keys::{ssh_key::Algorithm, PrivateKey};
        let key = PrivateKey::random(&mut rand::thread_rng(), Algorithm::Ed25519).unwrap();
        let plain = key.to_openssh(Default::default()).unwrap().as_bytes().to_vec();
        let encrypted = key.encrypt(&mut rand::thread_rng(), "fixture-passphrase").unwrap()
            .to_openssh(Default::default()).unwrap().as_bytes().to_vec();
        for (openssh, passphrase, expected) in [
            (plain, None, None),
            (encrypted.clone(), None, Some("keyPassphrase")),
            (encrypted.clone(), Some("wrong"), Some("keyPassphrase")),
            (encrypted, Some("fixture-passphrase"), None),
            (b"not a key".to_vec(), None, Some("keyParse")),
            (vec![0xff], None, Some("keyParse")),
            (b"PuTTY-User-Key-File-2: ssh-ed25519\nEncryption: aes256-cbc\n".to_vec(), None, Some("keyPassphrase")),
            (b"PuTTY-User-Key-File-2: unsupported\n".to_vec(), None, Some("keyParse")),
        ] {
            let material = PrivateKeyMaterial {
                openssh,
                passphrase: passphrase.map(|value| SecretString::new(value.into())),
            };
            assert_eq!(material.decode().err().map(|error| error.code()), expected);
        }
    }

    #[tokio::test]
    async fn manager_authenticator_rejects_invalid_private_key_passphrase_reference() {
        let credentials = CredentialState::with_store(Arc::new(TestCredentialStore::default()));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let key = directory.path().join("key");
        std::fs::write(&key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(directory.path(), &secrets, &credentials, vec![
            AuthMethodRef::PrivateKey {
                file_ref: key.to_string_lossy().into_owned(),
                passphrase_ref: Some("invalid-reference".into()),
            },
            AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
        ]);
        let mut context = recording_context(true);
        assert!(matches!(
            authenticator.authenticate(&mut context, "alice", &[]).await,
            Err(SshError::InvalidRequest(_))
        ));
        assert_eq!(context.calls, ["none"]);
    }

    #[tokio::test]
    async fn manager_authenticator_marks_private_key_after_password_rejection() {
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some(("ssh".into(), "alice".into(), "secret-password".into()));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![
                AuthMethodRef::Password {
                    secret_ref: "keychain://ssh/alice".into(),
                },
                private_key_method(&private_key),
            ],
        );
        let mut context = recording_context(true);

        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["password", "private-key:16"]);
        assert!(used_private_key(&authenticator));
    }

    #[tokio::test]
    async fn manager_authenticator_keeps_private_key_false_for_non_key_methods() {
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let directory = tempdir().unwrap();
        let cases: Vec<(&str, Vec<AuthMethodRef>, &str)> = vec![
            (
                "agent",
                vec![AuthMethodRef::Agent {
                    socket: Some("/tmp/agent.sock".into()),
                }],
                "agent:/tmp/agent.sock",
            ),
            (
                "keyboard-interactive",
                vec![AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }],
                "keyboard-interactive",
            ),
            ("none", Vec::new(), "none"),
        ];

        for (label, auth, expected_call) in cases {
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, auth);
            let mut context = recording_context(true);
            assert_eq!(authentication_succeeded(authenticator
                .authenticate(&mut context, "alice", &[])
                .await), label != "none");
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(context.calls, if label == "none" { vec![] } else { vec![expected_call] });
            assert!(
                !used_private_key(&authenticator),
                "{label} success must not report a private key"
            );
        }
    }

    #[tokio::test]
    async fn manager_authenticator_continues_after_rejected_agent() {
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some(("ssh".into(), "alice".into(), "secret-password".into()));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let agent = AuthMethodRef::Agent { socket: None };
        let password = AuthMethodRef::Password {
            secret_ref: "keychain://ssh/alice".into(),
        };
        let cases: Vec<(Vec<AuthMethodRef>, bool, Vec<&str>)> = vec![
            (
                vec![agent.clone(), AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }],
                true,
                vec!["agent:default", "keyboard-interactive"],
            ),
            (
                vec![agent.clone(), password],
                true,
                vec!["agent:default", "password"],
            ),
            (vec![agent], false, vec!["agent:default"]),
        ];

        for (auth, expected, expected_calls) in cases {
            let authenticator = authenticator_for(directory.path(), &secrets, &credentials, auth);
            let mut context = recording_context(true);
            context.password_accepted = true;
            context.agent_error = Some(|| SshError::AuthenticationRejected);
            assert_eq!(
                authentication_succeeded(authenticator
                    .authenticate(&mut context, "alice", &[])
                    .await),
                expected
            );
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(context.calls, expected_calls);
        }
    }

    #[tokio::test]
    async fn manager_authenticator_agent_success_and_fatal_errors_stop_later_methods() {
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let directory = tempdir().unwrap();
        let auth = vec![
            AuthMethodRef::Agent { socket: None },
            AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None },
        ];

        let authenticator =
            authenticator_for(directory.path(), &secrets, &credentials, auth.clone());
        let mut context = recording_context(true);
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["agent:default"]);

        let fatal: [(fn() -> SshError, &str); 2] =
            [(|| SshError::Closed, "closed"), (|| SshError::Timeout, "timeout")];
        for (error, code) in fatal {
            let authenticator =
                authenticator_for(directory.path(), &secrets, &credentials, auth.clone());
            let mut context = recording_context(true);
            context.agent_error = Some(error);
            let result = authenticator.authenticate(&mut context, "alice", &[]).await;
            assert_eq!(result.unwrap_err().code(), code);
            assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
            assert_eq!(context.calls, ["agent:default"]);
        }
    }

    #[tokio::test]
    async fn manager_authenticator_clears_private_key_on_a_later_authentication() {
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![private_key_method(&private_key)],
        );
        let mut context = recording_context(true);
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert!(used_private_key(&authenticator));

        let methods = [AuthMethodRef::KeyboardInteractive { password: None, secret_ref: None }];
        let mut context = recording_context(true);
        assert!(authenticator
            .authenticate(&mut context, "alice", &methods)
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["keyboard-interactive"]);
        assert!(!used_private_key(&authenticator));
    }

    #[tokio::test]
    async fn manager_authenticator_reports_false_when_private_key_does_not_succeed() {
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![
                private_key_method(&private_key),
                AuthMethodRef::Agent { socket: None },
            ],
        );
        let mut context = recording_context(false);
        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["private-key:16", "agent:default"]);
        assert!(!used_private_key(&authenticator));

        let unreadable = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![private_key_method(&directory.path().join("missing_key"))],
        );
        let mut context = recording_context(true);
        assert!(!authentication_succeeded(unreadable
            .authenticate(&mut context, "alice", &[])
            .await));
        assert_eq!(context.calls, ["none"]);
        assert!(!used_private_key(&unreadable));
    }

    #[tokio::test]
    async fn manager_authenticator_partial_private_key_then_password_marks_key_used() {
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some(("ssh".into(), "alice".into(), "secret-password".into()));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![
                private_key_method(&private_key),
                AuthMethodRef::Password {
                    secret_ref: "keychain://ssh/alice".into(),
                },
            ],
        );
        let mut context = recording_context(false);
        context.password_accepted = true;
        context.private_key_accepted = true;

        assert!(authenticator
            .authenticate(&mut context, "alice", &[])
            .await
            .unwrap());
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["private-key:16", "password"]);
        assert!(context.password_valid);
        assert!(used_private_key(&authenticator));
    }

    #[tokio::test]
    async fn manager_authenticator_partial_private_key_without_another_factor_is_rejected() {
        let mut credentials = TestCredentialStore::default();
        credentials.value = Some(("ssh".into(), "alice".into(), "secret-password".into()));
        let credentials = CredentialState::with_store(Arc::new(credentials));
        let secrets = SecretState::default();
        let directory = tempdir().unwrap();
        let private_key = directory.path().join("id_ed25519");
        std::fs::write(&private_key, b"test-private-key").unwrap();
        let authenticator = authenticator_for(
            directory.path(),
            &secrets,
            &credentials,
            vec![
                private_key_method(&private_key),
                AuthMethodRef::Password {
                    secret_ref: "keychain://ssh/alice".into(),
                },
            ],
        );
        let mut context = recording_context(false);
        context.private_key_accepted = true;

        assert!(!authentication_succeeded(authenticator
            .authenticate(&mut context, "alice", &[])
            .await));
        assert_eq!(context.calls.remove(0), "none", "Probe methods before credential attempts");
        assert_eq!(context.calls, ["private-key:16", "password"]);
        assert!(used_private_key(&authenticator));
    }

    #[test]
    fn private_key_acceptance_classifies_russh_auth_results() {
        let failure = |partial_success| AuthResult::Failure {
            remaining_methods: russh::MethodSet::empty(),
            partial_success,
        };
        assert!(public_key_accepted(&AuthResult::Success));
        assert!(!public_key_accepted(&failure(false)));
        assert!(public_key_accepted(&failure(true)));
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
