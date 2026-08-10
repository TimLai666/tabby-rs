pub mod engine;
mod import;
mod known_hosts;
pub mod model;

use std::{
    collections::HashMap,
    fs,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use russh::{
    client::{self, AuthResult, Handler, KeyboardInteractiveAuthResponse, Prompt},
    keys::{agent::client::AgentClient, decode_secret_key, PrivateKeyWithHashAlg},
    ChannelMsg, Disconnect,
};
use secrecy::ExposeSecret;
use tauri::{AppHandle, Emitter};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, oneshot},
    time::timeout,
};

use crate::{
    security::{
        CredentialAddress, CredentialNamespace, CredentialState, SecretState, VaultSecretSelector,
    },
    ssh::{
        known_hosts::fingerprint,
        model::{
            AuthMethodRef, HostKeyDecision, HostKeyDecisionRequest, HostKeyPrompt, HostKeyStatus,
            KeepaliveOptions, SshAuthPrompt, SshAuthPromptItem, SshAuthResponseRequest,
            SshConnectRequest, SshError, SshExitEvent, SshOutputEvent, SshResizeRequest,
            SshSessionIdRequest, SshSessionInfo, SshWriteRequest,
        },
    },
};

pub use import::*;
pub use known_hosts::KnownHostsStore;
pub use model::*;

const HOST_KEY_TIMEOUT: Duration = Duration::from_secs(60);
const AUTH_PROMPT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_BUFFER: usize = 16 * 1024 * 1024;

type HostKeySender = oneshot::Sender<HostKeyDecision>;
type AuthSender = oneshot::Sender<Vec<String>>;

#[derive(Clone)]
pub struct SshManager {
    known_hosts: KnownHostsStore,
    sessions: Arc<Mutex<HashMap<String, SshSession>>>,
    host_key_waiters: Arc<Mutex<HashMap<String, HostKeySender>>>,
    auth_waiters: Arc<Mutex<HashMap<String, AuthSender>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct SshSession {
    control: mpsc::Sender<SshControl>,
}

enum SshControl {
    Write(Vec<u8>, oneshot::Sender<Result<(), SshError>>),
    Resize(SshResizeRequest, oneshot::Sender<Result<(), SshError>>),
    Close(oneshot::Sender<Result<(), SshError>>),
}

struct SshHandler {
    manager: SshManager,
    host: String,
    port: u16,
    connection_id: String,
    app: AppHandle,
    host_key_error: Arc<Mutex<Option<SshError>>>,
}

impl Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self
            .manager
            .verify_host_key(
                &self.app,
                &self.host,
                self.port,
                &self.connection_id,
                server_public_key,
            )
            .await
        {
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
}

impl SshManager {
    pub fn new(known_hosts_path: std::path::PathBuf) -> Self {
        Self {
            known_hosts: KnownHostsStore::new(known_hosts_path),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            host_key_waiters: Arc::new(Mutex::new(HashMap::new())),
            auth_waiters: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
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
        let host_key_error = Arc::new(Mutex::new(None));
        let handler = SshHandler {
            manager: self.clone(),
            host: request.host.clone(),
            port: request.port,
            connection_id: connection_id.clone(),
            app: app.clone(),
            host_key_error: Arc::clone(&host_key_error),
        };
        let connection = timeout(
            Duration::from_secs(30),
            client::connect(
                Arc::new(config),
                (request.host.clone(), request.port),
                handler,
            ),
        )
        .await;
        let mut handle = match connection {
            Err(_) => return Err(SshError::Timeout),
            Ok(Ok(handle)) => handle,
            Ok(Err(_)) => {
                if let Some(error) = host_key_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
                {
                    return Err(error);
                }
                return Err(SshError::Connection);
            }
        };

        if !self
            .authenticate(
                &app,
                &mut handle,
                &request,
                &username,
                &secrets,
                &credentials,
            )
            .await?
        {
            let _ = handle
                .disconnect(
                    Disconnect::AuthCancelledByUser,
                    "authentication rejected",
                    "",
                )
                .await;
            return Err(SshError::AuthenticationRejected);
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|_| SshError::ChannelOpen)?;
        let terminal = &request.terminal;
        channel
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
            .map_err(|_| SshError::ChannelOpen)?;
        for (name, value) in &request.environment {
            channel
                .set_env(true, name, value)
                .await
                .map_err(|_| SshError::ChannelOpen)?;
        }
        channel
            .request_shell(true)
            .await
            .map_err(|_| SshError::ChannelOpen)?;

        let id = self.new_id("session");
        let (mut reader, writer) = channel.split();
        let (control, mut controls) = mpsc::channel(32);
        let sessions = Arc::clone(&self.sessions);
        let task_id = id.clone();
        let task_connection_id = connection_id.clone();
        let task_profile_id = request.profile_id.clone();
        let task_app = app.clone();
        let task_id_for_task = task_id.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    message = reader.wait() => {
                        let Some(message) = message else { break };
                        if !emit_channel_message(
                            &task_app,
                            &task_id_for_task,
                            &task_connection_id,
                            &task_profile_id,
                            message,
                        ) {
                            break;
                        }
                    }
                    control_message = controls.recv() => {
                        let Some(control_message) = control_message else { break };
                        if !handle_control(&mut handle, &writer, control_message).await {
                            break;
                        }
                    }
                }
            }
            sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&task_id_for_task);
            let _ = task_app.emit(
                "ssh.exit",
                SshExitEvent {
                    id: task_id_for_task,
                    connection_id: task_connection_id,
                    profile_id: task_profile_id,
                    exit_code: None,
                    signal: None,
                },
            );
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
                "ssh.hostKeyPrompt",
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
        match (status, decision) {
            (HostKeyStatus::Unknown, HostKeyDecision::Once) => Ok(true),
            (_, HostKeyDecision::Save) => self.known_hosts.save(host, port, key).map(|_| true),
            (HostKeyStatus::Changed, HostKeyDecision::Reject)
            | (HostKeyStatus::Changed, HostKeyDecision::Once) => Err(SshError::HostKeyChanged),
            (HostKeyStatus::Unknown, HostKeyDecision::Reject) => Err(SshError::HostKeyRejected),
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
        if request.auth.is_empty() {
            return Ok(matches!(
                handle
                    .authenticate_none(username)
                    .await
                    .map_err(|_| SshError::AuthenticationRejected)?,
                AuthResult::Success
            ));
        }
        for method in &request.auth {
            let result = match method {
                AuthMethodRef::Password { secret_ref } => {
                    let password = resolve_secret_ref(secret_ref, secrets, credentials)?;
                    handle
                        .authenticate_password(username, password.expose_secret())
                        .await
                        .map_err(|_| SshError::AuthenticationRejected)?
                }
                AuthMethodRef::PrivateKey {
                    file_ref,
                    passphrase_ref,
                } => {
                    let key = self
                        .load_private_key(
                            &app,
                            &request.profile_id,
                            request
                                .connection_id
                                .as_deref()
                                .unwrap_or(&request.profile_id),
                            file_ref,
                            passphrase_ref.as_deref(),
                            secrets,
                            credentials,
                        )
                        .await?;
                    handle
                        .authenticate_publickey(
                            username,
                            PrivateKeyWithHashAlg::new(Arc::new(key), None),
                        )
                        .await
                        .map_err(|_| SshError::AuthenticationRejected)?
                }
                AuthMethodRef::Agent { socket } => {
                    #[cfg(unix)]
                    {
                        let mut agent = match socket {
                            Some(path) => AgentClient::connect_uds(path)
                                .await
                                .map_err(|_| SshError::AuthenticationRejected)?,
                            None => AgentClient::connect_env()
                                .await
                                .map_err(|_| SshError::AuthenticationRejected)?,
                        };
                        let identities = agent
                            .request_identities()
                            .await
                            .map_err(|_| SshError::AuthenticationRejected)?;
                        let mut result = AuthResult::Failure {
                            remaining_methods: russh::MethodSet::empty(),
                            partial_success: false,
                        };
                        for identity in identities {
                            result = handle
                                .authenticate_publickey_with(username, identity, None, &mut agent)
                                .await
                                .map_err(|_| SshError::AuthenticationRejected)?;
                            if matches!(result, AuthResult::Success) {
                                break;
                            }
                        }
                        result
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = socket;
                        return Err(SshError::InvalidRequest(
                            "SSH agent authentication is unavailable on this platform".into(),
                        ));
                    }
                }
                AuthMethodRef::KeyboardInteractive => {
                    let mut response = handle
                        .authenticate_keyboard_interactive_start(username, None::<String>)
                        .await
                        .map_err(|_| SshError::AuthenticationRejected)?;
                    loop {
                        match response {
                            KeyboardInteractiveAuthResponse::Success => break AuthResult::Success,
                            KeyboardInteractiveAuthResponse::Failure {
                                remaining_methods,
                                partial_success,
                            } => {
                                break AuthResult::Failure {
                                    remaining_methods,
                                    partial_success,
                                }
                            }
                            KeyboardInteractiveAuthResponse::InfoRequest {
                                name,
                                instructions,
                                prompts,
                            } => {
                                let responses = self
                                    .prompt_for_responses(
                                        app,
                                        SshAuthPrompt {
                                            request_id: String::new(),
                                            id: request.profile_id.clone(),
                                            connection_id: request
                                                .connection_id
                                                .clone()
                                                .unwrap_or_else(|| request.profile_id.clone()),
                                            name,
                                            instructions,
                                            prompts: prompts.iter().map(prompt_item).collect(),
                                        },
                                    )
                                    .await?;
                                response = handle
                                    .authenticate_keyboard_interactive_respond(responses)
                                    .await
                                    .map_err(|_| SshError::AuthenticationRejected)?;
                            }
                        }
                    }
                }
            };
            if matches!(result, AuthResult::Success) {
                return Ok(true);
            }
        }
        Ok(false)
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
        if app.emit("ssh.authPrompt", prompt).is_err() {
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

    async fn load_private_key(
        &self,
        app: &AppHandle,
        profile_id: &str,
        connection_id: &str,
        file_ref: &str,
        passphrase_ref: Option<&str>,
        secrets: &SecretState,
        credentials: &CredentialState,
    ) -> Result<russh::keys::PrivateKey, SshError> {
        let bytes = if let Some(id) = file_ref.strip_prefix("vault://") {
            secrets.get_file(id).map_err(|_| SshError::KeyParse)?
        } else {
            fs::read(file_ref).map_err(|_| SshError::KeyParse)?
        };
        if bytes.len() > MAX_BUFFER {
            return Err(SshError::KeyParse);
        }
        let text = String::from_utf8(bytes).map_err(|_| SshError::KeyParse)?;
        let passphrase = passphrase_ref
            .map(|reference| resolve_secret_ref(reference, secrets, credentials))
            .transpose()?;
        let explicit = passphrase.as_ref().map(|value| value.expose_secret());
        let key = decode_secret_key(&text, explicit.map(|value| value.as_str()));
        match key {
            Ok(key) => Ok(key),
            Err(_) if passphrase_ref.is_none() => {
                let responses = self
                    .prompt_for_responses(
                        app,
                        SshAuthPrompt {
                            request_id: String::new(),
                            id: profile_id.into(),
                            connection_id: connection_id.into(),
                            name: "Private key passphrase".into(),
                            instructions: "The private key is encrypted.".into(),
                            prompts: vec![SshAuthPromptItem {
                                text: "Passphrase".into(),
                                echo: false,
                            }],
                        },
                    )
                    .await?;
                let passphrase = responses.first().ok_or(SshError::AuthenticationRejected)?;
                decode_secret_key(&text, Some(passphrase)).map_err(|_| SshError::KeyParse)
            }
            Err(_) => Err(SshError::KeyParse),
        }
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
        SshControl::Close(sender) => {
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

fn emit_channel_message(
    app: &AppHandle,
    id: &str,
    connection_id: &str,
    profile_id: &str,
    message: ChannelMsg,
) -> bool {
    let event = match message {
        ChannelMsg::Data { data } => app.emit(
            "ssh.output",
            SshOutputEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                data: data.to_vec(),
                extended: false,
            },
        ),
        ChannelMsg::ExtendedData { data, .. } => app.emit(
            "ssh.output",
            SshOutputEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                data: data.to_vec(),
                extended: true,
            },
        ),
        ChannelMsg::ExitStatus { exit_status } => app.emit(
            "ssh.exit",
            SshExitEvent {
                id: id.into(),
                connection_id: connection_id.into(),
                profile_id: profile_id.into(),
                exit_code: Some(exit_status),
                signal: None,
            },
        ),
        ChannelMsg::ExitSignal { signal_name, .. } => app.emit(
            "ssh.exit",
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

fn prompt_item(prompt: &Prompt) -> SshAuthPromptItem {
    SshAuthPromptItem {
        text: prompt.prompt.clone(),
        echo: prompt.echo,
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
        if *interval_ms < 100 || *interval_ms > 86_400_000 || *max_count > 100 {
            return Err(SshError::InvalidRequest(
                "SSH keepalive options are invalid".into(),
            ));
        }
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{model::*, validate_request};
    use std::collections::BTreeMap;

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
}
