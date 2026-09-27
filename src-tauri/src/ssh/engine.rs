use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;
use zeroize::Zeroize;

use crate::ssh::{AuthMethodRef, SshError};

const AUTH_TIMEOUT: Duration = Duration::from_secs(120);

/// The host-key information exposed at the engine boundary.
///
/// The OpenSSH representation is retained so a policy implementation can
/// compare and persist the exact key without depending on russh types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHostKey {
    pub algorithm: String,
    pub fingerprint_sha256: String,
    pub public_key_openssh: String,
}

#[async_trait::async_trait]
pub trait HostKeyVerifier: Send + Sync {
    async fn verify(&self, host: &str, port: u16, key: &SshHostKey) -> Result<bool, SshError>;
}

#[derive(Clone)]
pub struct PrivateKeyMaterial {
    pub openssh: Vec<u8>,
    pub passphrase: Option<SecretString>,
}

impl PrivateKeyMaterial {
    pub(super) fn decode(&self) -> Result<russh::keys::PrivateKey, SshError> {
        use russh::keys::{ssh_key, Error};
        let text = std::str::from_utf8(&self.openssh).map_err(|_| SshError::KeyParse)?;
        let passphrase = self
            .passphrase
            .as_ref()
            .map(|value| secrecy::ExposeSecret::expose_secret(value).as_str());
        russh::keys::decode_secret_key(text, passphrase).map_err(|error| match error {
            Error::KeyIsEncrypted | Error::SshKey(ssh_key::Error::Crypto) => {
                SshError::KeyPassphrase
            }
            // The pinned ssh-key fork does not export PpkParseError. Match only
            // the two variants upstream Tabby treats as passphrase failures.
            Error::SshKey(ssh_key::Error::Ppk(error))
                if matches!(format!("{error:?}").as_str(), "Encrypted" | "IncorrectMac") =>
            {
                SshError::KeyPassphrase
            }
            _ => SshError::KeyParse,
        })
    }
}

impl fmt::Debug for PrivateKeyMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateKeyMaterial")
            .field("openssh", &format_args!("<{} bytes>", self.openssh.len()))
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Drop for PrivateKeyMaterial {
    fn drop(&mut self) {
        self.openssh.zeroize();
    }
}

#[derive(Debug, Clone)]
pub struct KeyboardInteractivePrompt {
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<KeyboardInteractivePromptItem>,
}

#[derive(Debug, Clone)]
pub struct KeyboardInteractivePromptItem {
    pub text: String,
    pub echo: bool,
}

#[derive(Debug, Clone)]
pub enum KeyboardInteractiveResponse {
    Success,
    Failure,
    Prompt(KeyboardInteractivePrompt),
}

/// Auth methods use an engine-owned context so implementations can remain
/// independent of the selected SSH crate while still driving real auth.
#[async_trait::async_trait]
pub trait SshAuthContext: Send {
    /// Empty means the server has not restricted the next authentication method.
    fn remaining_auth_methods(&self) -> &[String] { &[] }
    async fn authenticate_none(&mut self, username: &str) -> Result<bool, SshError>;
    async fn authenticate_password(
        &mut self,
        username: &str,
        password: &SecretString,
    ) -> Result<bool, SshError>;
    async fn authenticate_private_key(
        &mut self,
        username: &str,
        key: PrivateKeyMaterial,
    ) -> Result<bool, SshError>;
    /// Reports whether the most recent `authenticate_private_key` attempt was
    /// accepted by the server, including the partial acceptance a server
    /// reports when another authentication factor is still required. The
    /// returned value is `false` for a rejected or unparsed key.
    fn private_key_was_accepted(&self) -> bool {
        false
    }
    async fn authenticate_agent(
        &mut self,
        username: &str,
        socket: Option<&str>,
    ) -> Result<bool, SshError>;
    async fn authenticate_keyboard_interactive_start(
        &mut self,
        username: &str,
    ) -> Result<KeyboardInteractiveResponse, SshError>;
    async fn authenticate_keyboard_interactive_respond(
        &mut self,
        responses: Vec<String>,
    ) -> Result<KeyboardInteractiveResponse, SshError>;
}

#[async_trait::async_trait]
pub trait SshAuthenticator: Send + Sync {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        methods: &[AuthMethodRef],
    ) -> Result<bool, SshError>;
}

#[derive(Debug, Clone)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct ShellChannelRequest {
    pub term: String,
    pub columns: u32,
    pub rows: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshChannelMessage {
    Data(Vec<u8>),
    ExtendedData { data: Vec<u8>, ext: u32 },
    ExitStatus(u32),
    ExitSignal { signal: String },
    Eof,
    Close,
}

#[async_trait::async_trait]
pub trait SshEngine: Send + Sync {
    async fn connect(
        &self,
        target: SshTarget,
        verifier: Arc<dyn HostKeyVerifier>,
        authenticator: Arc<dyn SshAuthenticator>,
    ) -> Result<Box<dyn SshConnection>, SshError>;
}

#[async_trait::async_trait]
pub trait SshConnection: Send + Sync {
    async fn open_shell(
        &self,
        request: ShellChannelRequest,
    ) -> Result<Box<dyn SshChannel>, SshError>;

    async fn disconnect(&self) -> Result<(), SshError>;
}

#[async_trait::async_trait]
pub trait SshChannel: Send + Sync {
    async fn read(&self) -> Result<Option<SshChannelMessage>, SshError>;
    async fn write(&self, data: &[u8]) -> Result<(), SshError>;
    async fn resize(
        &self,
        columns: u32,
        rows: u32,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Result<(), SshError>;
    async fn close(&self) -> Result<(), SshError>;
}

/// The production russh implementation of the crate-neutral engine boundary.
#[derive(Debug, Clone)]
pub struct RusshEngine {
    config: Arc<russh::client::Config>,
    connect_timeout: Duration,
    cancellation: super::pending::CancelSignal,
}

impl RusshEngine {
    pub fn new(config: russh::client::Config, connect_timeout: Duration) -> Self {
        Self {
            config: Arc::new(config),
            connect_timeout,
            cancellation: Default::default(),
        }
    }

    pub(crate) fn from_shared_config(
        config: Arc<russh::client::Config>,
        connect_timeout: Duration,
    ) -> Self {
        Self {
            config,
            connect_timeout,
            cancellation: Default::default(),
        }
    }

    pub(super) fn with_cancellation(mut self, signal: super::pending::CancelSignal) -> Self {
        self.cancellation = signal;
        self
    }

    pub(crate) async fn connect_with_handler<H>(
        &self,
        target: &SshTarget,
        handler: H,
        host_key_error: Arc<Mutex<Option<SshError>>>,
        authenticator: &dyn SshAuthenticator,
    ) -> Result<russh::client::Handle<H>, SshError>
    where
        H: russh::client::Handler<Error = russh::Error> + Send + 'static,
    {
        let mut handle = match tokio::time::timeout(
            self.connect_timeout,
            async {
                let stream = tokio::net::TcpStream::connect((target.host.as_str(), target.port)).await?;
                if self.config.nodelay {
                    if let Err(error) = stream.set_nodelay(true) {
                        eprintln!("SSH TCP_NODELAY failed: {error}");
                    }
                }
                russh::client::connect_stream(
                    Arc::clone(&self.config), self.cancellation.wrap(stream), handler,
                ).await
            },
        )
        .await
        {
            Err(_) => return Err(SshError::Timeout),
            Ok(Ok(handle)) => handle,
            Ok(Err(_)) => {
                return Err(host_key_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
                    .unwrap_or(SshError::Connection))
            }
        };
        let authenticated = match tokio::time::timeout(
            AUTH_TIMEOUT,
            authenticate_handle(&mut handle, &target.username, authenticator),
        )
        .await
        {
            Err(_) => {
                let _ = handle
                    .disconnect(
                        russh::Disconnect::AuthCancelledByUser,
                        "authentication timed out",
                        "",
                    )
                    .await;
                return Err(SshError::Timeout);
            }
            Ok(Ok(authenticated)) => authenticated,
            Ok(Err(error)) => {
                let _ = handle
                    .disconnect(
                        russh::Disconnect::AuthCancelledByUser,
                        "authentication failed",
                        "",
                    )
                    .await;
                return Err(error);
            }
        };
        if !authenticated {
            let _ = handle
                .disconnect(
                    russh::Disconnect::AuthCancelledByUser,
                    "authentication rejected",
                    "",
                )
                .await;
            return Err(SshError::AuthenticationRejected);
        }
        Ok(handle)
    }
}

struct RusshHandler {
    verifier: Arc<dyn HostKeyVerifier>,
    host: String,
    port: u16,
    host_key_error: Arc<Mutex<Option<SshError>>>,
}

impl russh::client::Handler for RusshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let key = SshHostKey {
            algorithm: format!("{:?}", server_public_key.algorithm()),
            fingerprint_sha256: server_public_key
                .fingerprint(russh::keys::HashAlg::Sha256)
                .to_string(),
            public_key_openssh: server_public_key
                .to_openssh()
                .map_err(|_| russh::Error::UnknownKey)?,
        };
        match self.verifier.verify(&self.host, self.port, &key).await {
            Ok(true) => Ok(true),
            Ok(false) => {
                *self
                    .host_key_error
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    Some(SshError::HostKeyRejected);
                Ok(false)
            }
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

/// Reports whether the server accepted a public key attempt, including the
/// partial acceptance a server returns when another factor is still required.
pub(super) fn public_key_accepted(result: &russh::client::AuthResult) -> bool {
    matches!(
        result,
        russh::client::AuthResult::Success
            | russh::client::AuthResult::Failure {
                partial_success: true,
                ..
            }
    )
}

pub(super) struct RusshAuthContext<'a, H: russh::client::Handler> {
    handle: &'a mut russh::client::Handle<H>,
    private_key_accepted: bool,
    remaining_methods: Vec<String>,
}

impl<'a, H: russh::client::Handler> RusshAuthContext<'a, H> {
    pub(super) fn new(handle: &'a mut russh::client::Handle<H>) -> Self {
        Self { handle, private_key_accepted: false, remaining_methods: Vec::new() }
    }

    fn remember_methods(&mut self, methods: &russh::MethodSet) {
        if !methods.is_empty() {
            self.remaining_methods = methods.iter().map(String::from).collect();
        }
    }

    fn remember_result(&mut self, result: &russh::client::AuthResult) {
        if let russh::client::AuthResult::Failure { remaining_methods, .. } = result {
            self.remember_methods(remaining_methods);
        }
    }

    fn keyboard_response(&mut self, response: russh::client::KeyboardInteractiveAuthResponse) -> Result<KeyboardInteractiveResponse, SshError> {
        if let russh::client::KeyboardInteractiveAuthResponse::Failure { remaining_methods, .. } = &response {
            self.remember_methods(remaining_methods);
        }
        map_keyboard_interactive(response)
    }
}

#[async_trait::async_trait]
impl<H> SshAuthContext for RusshAuthContext<'_, H>
where
    H: russh::client::Handler<Error = russh::Error> + Send,
{
    fn remaining_auth_methods(&self) -> &[String] { &self.remaining_methods }

    async fn authenticate_none(&mut self, username: &str) -> Result<bool, SshError> {
        let result = self.handle
                .authenticate_none(username)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?;
        self.remember_result(&result);
        Ok(result.success())
    }

    async fn authenticate_password(
        &mut self,
        username: &str,
        password: &SecretString,
    ) -> Result<bool, SshError> {
        let result = self.handle
                .authenticate_password(username, secrecy::ExposeSecret::expose_secret(password))
                .await
                .map_err(|_| SshError::AuthenticationRejected)?;
        self.remember_result(&result);
        Ok(result.success())
    }

    async fn authenticate_private_key(
        &mut self,
        username: &str,
        key: PrivateKeyMaterial,
    ) -> Result<bool, SshError> {
        self.private_key_accepted = false;
        let private_key = key.decode()?;
        let result = self
            .handle
            .authenticate_publickey(
                username,
                russh::keys::PrivateKeyWithHashAlg::new(Arc::new(private_key), None),
            )
            .await
            .map_err(|_| SshError::AuthenticationRejected)?;
        self.private_key_accepted = public_key_accepted(&result);
        self.remember_result(&result);
        Ok(result.success())
    }

    fn private_key_was_accepted(&self) -> bool {
        self.private_key_accepted
    }

    async fn authenticate_agent(
        &mut self,
        username: &str,
        socket: Option<&str>,
    ) -> Result<bool, SshError> {
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (username, socket);
            return Err(SshError::AuthenticationRejected);
        }

        #[cfg(any(unix, windows))]
        {
            let mut agent = super::connect_agent(socket.map(str::to_owned)).await?;
            let identities = agent
                .request_identities()
                .await
                .map_err(|_| SshError::AuthenticationRejected)?;
            for identity in identities {
                let result = self
                    .handle
                    .authenticate_publickey_with(username, identity, None, &mut agent)
                    .await
                    .map_err(|_| SshError::AuthenticationRejected)?;
                if matches!(result, russh::client::AuthResult::Success) {
                    return Ok(true);
                }
                self.remember_result(&result);
                if !self.remaining_methods.is_empty() && !self.remaining_methods.iter().any(|method| method == "publickey") {
                    break;
                }
            }
        }
        Ok(false)
    }

    async fn authenticate_keyboard_interactive_start(
        &mut self,
        username: &str,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        let response = self.handle
                .authenticate_keyboard_interactive_start(username, None::<String>)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?;
        self.keyboard_response(response)
    }

    async fn authenticate_keyboard_interactive_respond(
        &mut self,
        responses: Vec<String>,
    ) -> Result<KeyboardInteractiveResponse, SshError> {
        let response = self.handle
                .authenticate_keyboard_interactive_respond(responses)
                .await
                .map_err(|_| SshError::AuthenticationRejected)?;
        self.keyboard_response(response)
    }
}

fn map_keyboard_interactive(
    response: russh::client::KeyboardInteractiveAuthResponse,
) -> Result<KeyboardInteractiveResponse, SshError> {
    Ok(match response {
        russh::client::KeyboardInteractiveAuthResponse::Success => {
            KeyboardInteractiveResponse::Success
        }
        russh::client::KeyboardInteractiveAuthResponse::Failure { .. } => {
            KeyboardInteractiveResponse::Failure
        }
        russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
            name,
            instructions,
            prompts,
        } => KeyboardInteractiveResponse::Prompt(KeyboardInteractivePrompt {
            name,
            instructions,
            prompts: prompts
                .into_iter()
                .map(|prompt| KeyboardInteractivePromptItem {
                    text: prompt.prompt,
                    echo: prompt.echo,
                })
                .collect(),
        }),
    })
}

async fn authenticate_handle<H>(
    handle: &mut russh::client::Handle<H>,
    username: &str,
    authenticator: &dyn SshAuthenticator,
) -> Result<bool, SshError>
where
    H: russh::client::Handler<Error = russh::Error> + Send,
{
    let mut context = RusshAuthContext::new(handle);
    authenticator
        .authenticate(&mut context, username, &[])
        .await
}

#[async_trait::async_trait]
impl SshEngine for RusshEngine {
    async fn connect(
        &self,
        target: SshTarget,
        verifier: Arc<dyn HostKeyVerifier>,
        authenticator: Arc<dyn SshAuthenticator>,
    ) -> Result<Box<dyn SshConnection>, SshError> {
        let host_key_error = Arc::new(Mutex::new(None));
        let handler = RusshHandler {
            verifier,
            host: target.host.clone(),
            port: target.port,
            host_key_error: Arc::clone(&host_key_error),
        };
        let handle = self
            .connect_with_handler(&target, handler, host_key_error, authenticator.as_ref())
            .await?;

        Ok(Box::new(RusshConnection {
            handle: AsyncMutex::new(Some(handle)),
        }))
    }
}

struct RusshConnection {
    handle: AsyncMutex<Option<russh::client::Handle<RusshHandler>>>,
}

#[async_trait::async_trait]
impl SshConnection for RusshConnection {
    async fn open_shell(
        &self,
        request: ShellChannelRequest,
    ) -> Result<Box<dyn SshChannel>, SshError> {
        let handle = self.handle.lock().await;
        let handle = handle.as_ref().ok_or(SshError::Closed)?;
        let mut pending = VecDeque::new();
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|_| SshError::ChannelOpen)?;
        channel
            .request_pty(
                true,
                &request.term,
                request.columns,
                request.rows,
                request.pixel_width,
                request.pixel_height,
                &[],
            )
            .await
            .map_err(|_| SshError::ChannelOpen)?;
        pending.extend(super::wait_for_channel_confirmation(&mut channel).await?);
        for (name, value) in request.environment {
            channel
                .set_env(true, name, value)
                .await
                .map_err(|_| SshError::ChannelOpen)?;
            pending.extend(super::wait_for_channel_confirmation(&mut channel).await?);
        }
        super::start_shell_channel(&mut channel, false, false).await?;
        let (reader, writer) = channel.split();
        Ok(Box::new(RusshChannel {
            reader: AsyncMutex::new(reader),
            writer: AsyncMutex::new(writer),
            pending: AsyncMutex::new(pending),
        }))
    }

    async fn disconnect(&self) -> Result<(), SshError> {
        let mut handle = self.handle.lock().await;
        let Some(handle) = handle.take() else {
            return Ok(());
        };
        handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await
            .map_err(|_| SshError::Closed)
    }
}

struct RusshChannel {
    reader: AsyncMutex<russh::ChannelReadHalf>,
    writer: AsyncMutex<russh::ChannelWriteHalf<russh::client::Msg>>,
    pending: AsyncMutex<VecDeque<russh::ChannelMsg>>,
}

fn map_channel_message(message: russh::ChannelMsg) -> Option<SshChannelMessage> {
    match message {
        russh::ChannelMsg::Data { data } => Some(SshChannelMessage::Data(data.to_vec())),
        russh::ChannelMsg::ExtendedData { ext, data } => Some(SshChannelMessage::ExtendedData {
            ext,
            data: data.to_vec(),
        }),
        russh::ChannelMsg::ExitStatus { exit_status } => {
            Some(SshChannelMessage::ExitStatus(exit_status))
        }
        russh::ChannelMsg::ExitSignal { signal_name, .. } => Some(SshChannelMessage::ExitSignal {
            signal: format!("{signal_name:?}"),
        }),
        russh::ChannelMsg::Eof => Some(SshChannelMessage::Eof),
        russh::ChannelMsg::Close => Some(SshChannelMessage::Close),
        _ => None,
    }
}

#[async_trait::async_trait]
impl SshChannel for RusshChannel {
    async fn read(&self) -> Result<Option<SshChannelMessage>, SshError> {
        loop {
            let message = self.pending.lock().await.pop_front();
            let message = match message {
                Some(message) => Some(message),
                None => self.reader.lock().await.wait().await,
            };
            let Some(message) = message else {
                return Ok(None);
            };
            if let Some(message) = map_channel_message(message) {
                return Ok(Some(message));
            }
        }
    }

    async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        self.writer
            .lock()
            .await
            .data(Cursor::new(data.to_vec()))
            .await
            .map_err(|_| SshError::Closed)
    }

    async fn resize(
        &self,
        columns: u32,
        rows: u32,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Result<(), SshError> {
        self.writer
            .lock()
            .await
            .window_change(columns, rows, pixel_width, pixel_height)
            .await
            .map_err(|_| SshError::Closed)
    }

    async fn close(&self) -> Result<(), SshError> {
        self.writer
            .lock()
            .await
            .close()
            .await
            .map_err(|_| SshError::Closed)
    }
}
