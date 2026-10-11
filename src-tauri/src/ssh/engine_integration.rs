use std::{borrow::Cow, sync::Arc, time::Duration};

#[cfg(unix)]
use std::{fs, process::Command};

use russh::{
    client,
    keys::{parse_public_key_base64, PrivateKey, PublicKey},
    server::{self, Auth, Handler as ServerHandler, Response, Server},
    SshId,
};
use secrecy::{ExposeSecret, SecretString};

#[cfg(unix)]
use tempfile::tempdir;

use super::engine::{
    HostKeyVerifier, KeyboardInteractiveResponse, SshAuthContext, SshAuthenticator, SshEngine,
    SshHostKey, SshTarget,
};

struct FixtureHostKeyVerifier;

#[async_trait::async_trait]
impl HostKeyVerifier for FixtureHostKeyVerifier {
    async fn verify(
        &self,
        _host: &str,
        _port: u16,
        _key: &SshHostKey,
    ) -> Result<bool, crate::ssh::SshError> {
        Ok(true)
    }
}

#[derive(Clone, Copy)]
enum AuthFixtureKind {
    Agent,
    Password,
    PromptPassword,
    KeyboardInteractive,
    PrivateKey,
}

struct AuthFixtureServer {
    kind: AuthFixtureKind,
    expected: String,
    authorized_public_key: Option<String>,
}

impl Server for AuthFixtureServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self {
        Self {
            kind: self.kind,
            expected: self.expected.clone(),
            authorized_public_key: self.authorized_public_key.clone(),
        }
    }
}

impl ServerHandler for AuthFixtureServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if matches!(
            self.kind,
            AuthFixtureKind::Password | AuthFixtureKind::PromptPassword
        ) && password == self.expected
        {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if self.public_key_matches(public_key) {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        if self.public_key_matches(public_key) {
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if !matches!(self.kind, AuthFixtureKind::KeyboardInteractive) {
            return Ok(Auth::reject());
        }

        match response {
            None => Ok(Auth::Partial {
                name: Cow::Borrowed("tabby-rs fixture"),
                instructions: Cow::Borrowed("enter the fixture secret"),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Secret: "), false)]),
            }),
            Some(mut response) => {
                let received = response
                    .next()
                    .map(|value| String::from_utf8_lossy(&value).into_owned());
                if received.as_deref() == Some(self.expected.as_str()) {
                    Ok(Auth::Accept)
                } else {
                    Ok(Auth::reject())
                }
            }
        }
    }
}

impl AuthFixtureServer {
    fn public_key_matches(&self, public_key: &PublicKey) -> bool {
        if !matches!(
            self.kind,
            AuthFixtureKind::Agent | AuthFixtureKind::PrivateKey
        ) {
            return false;
        }
        let Ok(public_key) = public_key.to_openssh() else {
            return false;
        };
        self.authorized_public_key.as_deref() == Some(public_key.as_str())
    }
}

struct AuthFixtureAuthenticator {
    agent_socket: Option<String>,
    kind: AuthFixtureKind,
    expected: SecretString,
    private_key: Option<super::engine::PrivateKeyMaterial>,
}

#[derive(Clone, Copy)]
enum HostKeyAlgorithm {
    Ed25519,
    RsaSha256,
    EcdsaP256,
}

impl HostKeyAlgorithm {
    fn generate(self) -> russh::keys::PrivateKey {
        let algorithm = match self {
            Self::Ed25519 => russh::keys::Algorithm::Ed25519,
            Self::RsaSha256 => russh::keys::Algorithm::Rsa {
                hash: Some(russh::keys::HashAlg::Sha256),
            },
            Self::EcdsaP256 => russh::keys::Algorithm::Ecdsa {
                curve: russh::keys::EcdsaCurve::NistP256,
            },
        };
        russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, algorithm)
            .expect("generate russh fixture host key")
    }
}

#[async_trait::async_trait]
impl SshAuthenticator for AuthFixtureAuthenticator {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        _methods: &[crate::ssh::AuthMethodRef],
    ) -> Result<bool, crate::ssh::SshError> {
        match self.kind {
            AuthFixtureKind::Agent => {
                context
                    .authenticate_agent(username, self.agent_socket.as_deref())
                    .await
            }
            AuthFixtureKind::Password => {
                context
                    .authenticate_password(username, &self.expected)
                    .await
            }
            AuthFixtureKind::PromptPassword => {
                super::authenticate_password_response(
                    context,
                    username,
                    vec![self.expected.expose_secret().clone()],
                )
                .await
            }
            AuthFixtureKind::PrivateKey => {
                context
                    .authenticate_private_key(
                        username,
                        self.private_key
                            .clone()
                            .expect("private-key fixture material"),
                    )
                    .await
            }
            AuthFixtureKind::KeyboardInteractive => {
                let mut response = context
                    .authenticate_keyboard_interactive_start(username)
                    .await?;
                loop {
                    match response {
                        KeyboardInteractiveResponse::Success => return Ok(true),
                        KeyboardInteractiveResponse::Failure => return Ok(false),
                        KeyboardInteractiveResponse::Prompt(prompt) => {
                            assert_eq!(prompt.prompts.len(), 1);
                            response = context
                                .authenticate_keyboard_interactive_respond(vec![self
                                    .expected
                                    .expose_secret()
                                    .to_owned()])
                                .await?;
                        }
                    }
                }
            }
        }
    }
}

fn private_key_fixture(encrypted: bool) -> (super::engine::PrivateKeyMaterial, PublicKey) {
    let key = PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
        .expect("generate russh fixture client key");
    let public_key = key.public_key().clone();
    let mut openssh = Vec::new();
    if encrypted {
        russh::keys::encode_pkcs8_pem_encrypted(&key, b"fixture-passphrase", 1, &mut openssh)
            .expect("encode encrypted russh fixture client key");
    } else {
        russh::keys::encode_pkcs8_pem(&key, &mut openssh).expect("encode russh fixture client key");
    }
    (
        super::engine::PrivateKeyMaterial {
            openssh,
            passphrase: encrypted.then(|| SecretString::new("fixture-passphrase".into())),
        },
        public_key,
    )
}

async fn run_russh_auth_fixture(
    agent_socket: Option<String>,
    kind: AuthFixtureKind,
    host_key: russh::keys::PrivateKey,
    private_key: Option<super::engine::PrivateKeyMaterial>,
    authorized_public_key: Option<PublicKey>,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind russh auth fixture");
    let port = listener
        .local_addr()
        .expect("read russh fixture address")
        .port();
    let mut config = server::Config::default();
    config.server_id = SshId::Standard("SSH-2.0-tabby-rs-auth-fixture".into());
    config.keys.push(host_key);
    let config = Arc::new(config);
    let expected = "fixture-secret";
    let mut server = AuthFixtureServer {
        kind,
        expected: expected.into(),
        authorized_public_key: authorized_public_key
            .map(|key| key.to_openssh().expect("encode fixture public key")),
    };
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept russh fixture");
        let session = server::run_stream(config, socket, server.new_client(None))
            .await
            .expect("start russh fixture session");
        session.await.expect("run russh fixture session")
    });

    let engine = super::engine::RusshEngine::new(
        client::Config {
            inactivity_timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        },
        Duration::from_secs(20),
    );
    let connection = match tokio::time::timeout(
        Duration::from_secs(15),
        engine.connect(
            SshTarget {
                host: "127.0.0.1".into(),
                port,
                username: "fixture-user".into(),
            },
            Arc::new(FixtureHostKeyVerifier),
            Arc::new(AuthFixtureAuthenticator {
                agent_socket,
                kind,
                expected: SecretString::new(expected.into()),
                private_key,
            }),
        ),
    )
    .await
    {
        Ok(Ok(connection)) => connection,
        Ok(Err(error)) => {
            server_task.abort();
            panic!("russh auth fixture connection failed: {error:?}");
        }
        Err(_) => {
            server_task.abort();
            panic!("russh auth fixture connection timed out");
        }
    };
    match tokio::time::timeout(Duration::from_secs(5), connection.disconnect()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            server_task.abort();
            panic!("disconnect russh auth fixture failed: {error:?}");
        }
        Err(_) => {
            server_task.abort();
            panic!("disconnect russh auth fixture timed out");
        }
    }
    match tokio::time::timeout(Duration::from_secs(5), server_task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("join russh auth fixture failed: {error}"),
        Err(_) => panic!("join russh auth fixture timed out"),
    }
}

#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn runs_real_authentication_and_host_key_algorithm_matrix() {
    for host_key_algorithm in [
        HostKeyAlgorithm::Ed25519,
        HostKeyAlgorithm::RsaSha256,
        HostKeyAlgorithm::EcdsaP256,
    ] {
        let host_key = host_key_algorithm.generate();
        run_russh_auth_fixture(
            None,
            AuthFixtureKind::Password,
            host_key.clone(),
            None,
            None,
        )
        .await;
        run_russh_auth_fixture(
            None,
            AuthFixtureKind::PromptPassword,
            host_key.clone(),
            None,
            None,
        )
        .await;
        run_russh_auth_fixture(
            None,
            AuthFixtureKind::KeyboardInteractive,
            host_key.clone(),
            None,
            None,
        )
        .await;
        for encrypted in [false, true] {
            let (private_key, public_key) = private_key_fixture(encrypted);
            run_russh_auth_fixture(
                None,
                AuthFixtureKind::PrivateKey,
                host_key.clone(),
                Some(private_key),
                Some(public_key),
            )
            .await;
        }
    }
}

#[cfg(unix)]
struct SshAgentGuard {
    pid: String,
    socket: String,
}

#[cfg(unix)]
impl Drop for SshAgentGuard {
    fn drop(&mut self) {
        let _ = Command::new("ssh-agent")
            .arg("-k")
            .env("SSH_AGENT_PID", &self.pid)
            .env("SSH_AUTH_SOCK", &self.socket)
            .status();
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn runs_real_ssh_agent_authentication() {
    let directory = tempdir().expect("create SSH agent fixture directory");
    let key_path = directory.path().join("id_ed25519");
    let generated = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key_path)
        .status()
        .expect("run ssh-keygen");
    assert!(
        generated.success(),
        "ssh-keygen failed to create fixture key"
    );
    let public_key_text = fs::read_to_string(key_path.with_extension("pub"))
        .expect("read SSH agent fixture public key");
    let public_key = parse_public_key_base64(
        public_key_text
            .split_whitespace()
            .nth(1)
            .expect("SSH agent fixture public key is malformed"),
    )
    .expect("parse SSH agent fixture public key");

    let output = Command::new("ssh-agent")
        .arg("-s")
        .output()
        .expect("start ssh-agent");
    assert!(output.status.success(), "ssh-agent failed to start");
    let output = String::from_utf8(output.stdout).expect("ssh-agent output is UTF-8");
    let socket = output
        .lines()
        .find_map(|line| line.strip_prefix("SSH_AUTH_SOCK=")?.split(';').next())
        .map(str::to_owned)
        .expect("ssh-agent did not report SSH_AUTH_SOCK");
    let pid = output
        .lines()
        .find_map(|line| line.strip_prefix("SSH_AGENT_PID=")?.split(';').next())
        .map(str::to_owned)
        .expect("ssh-agent did not report SSH_AGENT_PID");
    let _agent = SshAgentGuard {
        pid,
        socket: socket.clone(),
    };
    let added = Command::new("ssh-add")
        .arg(&key_path)
        .env("SSH_AUTH_SOCK", &socket)
        .status()
        .expect("run ssh-add");
    assert!(added.success(), "ssh-add failed to load the fixture key");

    run_russh_auth_fixture(
        Some(socket),
        AuthFixtureKind::Agent,
        HostKeyAlgorithm::Ed25519.generate(),
        None,
        Some(public_key),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn classifies_dns_and_tcp_faults_with_bounded_timeouts() {
    let engine =
        super::engine::RusshEngine::new(client::Config::default(), Duration::from_millis(150));
    let verifier: Arc<dyn HostKeyVerifier> = Arc::new(FixtureHostKeyVerifier);
    let authenticator: Arc<dyn SshAuthenticator> = Arc::new(AuthFixtureAuthenticator {
        agent_socket: None,
        kind: AuthFixtureKind::Password,
        expected: SecretString::new("fixture-secret".into()),
        private_key: None,
    });

    let dns_failure = engine
        .connect(
            SshTarget {
                host: "ssh-fault.invalid".into(),
                port: 22,
                username: "fixture-user".into(),
            },
            Arc::clone(&verifier),
            Arc::clone(&authenticator),
        )
        .await;
    assert!(matches!(dns_failure, Err(crate::ssh::SshError::Connection)));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let timeout = engine
        .connect(
            SshTarget {
                host: "127.0.0.1".into(),
                port,
                username: "fixture-user".into(),
            },
            verifier,
            authenticator,
        )
        .await;
    assert!(matches!(timeout, Err(crate::ssh::SshError::Timeout)));
    server.abort();
}

#[derive(Clone, Copy)]
enum ForwardedChannel {
    #[cfg(unix)]
    Agent,
    #[cfg(unix)]
    X11,
    #[cfg(unix)]
    X11Unavailable,
    X11Tcp,
}

struct ForwardingConsentClient {
    host_key: PublicKey,
    socket: String,
    enabled: bool,
    messages: Arc<std::sync::Mutex<Vec<String>>>,
    forwards: tokio::sync::mpsc::UnboundedSender<tokio::task::JoinHandle<Result<(), russh::Error>>>,
}

impl client::Handler for ForwardingConsentClient {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.host_key.key_data())
    }

    fn server_channel_open_agent_forward(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let _ = self
            .forwards
            .send(tokio::spawn(super::forward_agent_channel(
                channel,
                Some(self.socket.clone()),
                self.enabled,
            )));
        async { Ok(()) }
    }

    fn server_channel_open_x11(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let messages = Arc::clone(&self.messages);
        let _ = self.forwards.send(tokio::spawn(super::forward_x11_channel(
            channel,
            Some(self.socket.clone()),
            self.enabled,
            move |message| messages.lock().unwrap().push(message),
        )));
        async { Ok(()) }
    }
}

async fn run_forwarding_consent(forwarded: ForwardedChannel, kind: AuthFixtureKind, enabled: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const IO: Duration = Duration::from_secs(5);
    const REQUEST_IDENTITIES: [u8; 5] = [0, 0, 0, 1, 11];
    const IDENTITIES_ANSWER: [u8; 9] = [0, 0, 0, 5, 12, 0, 0, 0, 0];

    async fn respond(
        mut stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
        accepted: Arc<AtomicUsize>,
    ) {
        accepted.fetch_add(1, Ordering::SeqCst);
        let mut request = [0; 5];
        stream
            .read_exact(&mut request)
            .await
            .expect("read agent request");
        assert_eq!(request, REQUEST_IDENTITIES);
        stream
            .write_all(&IDENTITIES_ANSWER)
            .await
            .expect("write agent answer");
        let _ = stream.read(&mut [0; 1]).await;
    }

    let mut tasks = tokio::task::JoinSet::new();
    #[cfg(unix)]
    let directory = tempdir().expect("create forwarding consent directory");
    let accepted = Arc::new(AtomicUsize::new(0));
    let messages = Arc::new(std::sync::Mutex::new(Vec::new()));
    #[cfg(unix)]
    let unavailable = matches!(forwarded, ForwardedChannel::X11Unavailable);
    #[cfg(not(unix))]
    let unavailable = false;
    let local_accepted = Arc::clone(&accepted);
    let socket = match forwarded {
        #[cfg(unix)]
        ForwardedChannel::X11Unavailable => directory
            .path()
            .join("missing.sock")
            .to_string_lossy()
            .into_owned(),
        ForwardedChannel::X11Tcp => {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tasks.spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                respond(stream, local_accepted).await;
            });
            format!("127.0.0.1:{port}.0")
        }
        #[cfg(unix)]
        _ => {
            let path = directory.path().join("forward.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            tasks.spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                respond(stream, local_accepted).await;
            });
            path.to_string_lossy().into_owned()
        }
    };

    let host_key = HostKeyAlgorithm::Ed25519.generate();
    let host_public_key = host_key.public_key().clone();
    let client_key = PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
        .expect("generate agent consent client key");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind agent consent fixture");
    let port = listener.local_addr().expect("read fixture address").port();
    let mut config = server::Config::default();
    config.keys.push(host_key);
    let mut server = AuthFixtureServer {
        kind,
        expected: "fixture-secret".into(),
        authorized_public_key: Some(client_key.public_key().to_openssh().expect("encode key")),
    };
    let (server_handle, server_handle_rx) = tokio::sync::oneshot::channel();
    tasks.spawn(async move {
        let (tcp, _) = listener
            .accept()
            .await
            .expect("accept agent consent fixture");
        let session = server::run_stream(Arc::new(config), tcp, server.new_client(None))
            .await
            .expect("start agent consent fixture");
        let _ = server_handle.send(session.handle());
        let _ = session.await;
    });

    let (forwards, mut forward_tasks) = tokio::sync::mpsc::unbounded_channel();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect agent consent fixture");
    let handler = ForwardingConsentClient {
        host_key: host_public_key,
        socket,
        enabled,
        messages: Arc::clone(&messages),
        forwards,
    };
    let mut client = tokio::time::timeout(
        IO,
        client::connect_stream(Arc::new(client::Config::default()), tcp, handler),
    )
    .await
    .expect("client handshake timed out")
    .expect("client handshake failed");
    let authenticated = match kind {
        AuthFixtureKind::Password => {
            tokio::time::timeout(
                IO,
                client.authenticate_password("fixture-user", "fixture-secret"),
            )
            .await
        }
        AuthFixtureKind::PrivateKey => {
            let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(client_key), None);
            tokio::time::timeout(IO, client.authenticate_publickey("fixture-user", key)).await
        }
        _ => unreachable!("agent consent covers password and private-key login"),
    };
    assert!(authenticated
        .expect("authentication timed out")
        .expect("authentication failed")
        .success());

    let server_handle = tokio::time::timeout(IO, server_handle_rx)
        .await
        .expect("server handle timed out")
        .expect("server handle dropped");
    let channel = match forwarded {
        #[cfg(unix)]
        ForwardedChannel::Agent => {
            tokio::time::timeout(IO, server_handle.channel_open_agent()).await
        }
        #[cfg(unix)]
        ForwardedChannel::X11 | ForwardedChannel::X11Unavailable => {
            tokio::time::timeout(IO, server_handle.channel_open_x11("127.0.0.1", 0)).await
        }
        ForwardedChannel::X11Tcp => {
            tokio::time::timeout(IO, server_handle.channel_open_x11("127.0.0.1", 0)).await
        }
    }
    .expect("open unsolicited channel timed out")
    .expect("open unsolicited channel failed");
    let mut stream = channel.into_stream();
    let written = stream.write_all(&REQUEST_IDENTITIES).await;
    if enabled && !unavailable {
        written.expect("send agent request over channel");
        let mut answer = [0; 9];
        tokio::time::timeout(IO, stream.read_exact(&mut answer))
            .await
            .expect("agent answer timed out")
            .expect("read agent answer");
        assert_eq!(answer, IDENTITIES_ANSWER);
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    } else {
        let mut buffer = [0; 16];
        let read = tokio::time::timeout(IO, stream.read(&mut buffer))
            .await
            .expect("disabled or failed forwarding must close the channel, not time out");
        assert!(
            matches!(read, Ok(0) | Err(_)),
            "disabled forwarding reached the local socket: {:?}",
            read.map(|count| buffer[..count].to_vec())
        );
        assert_eq!(accepted.load(Ordering::SeqCst), 0);
    }
    {
        let messages = messages.lock().unwrap();
        if enabled && unavailable {
            assert_eq!(messages.len(), 2);
            assert!(messages[0].contains("Could not connect to the X server"));
            assert!(messages[1].contains("missing.sock"));
        } else {
            assert!(
                messages.is_empty(),
                "successful and disabled channels must be silent: {messages:?}"
            );
        }
    }

    let _ = tokio::time::timeout(
        IO,
        client.disconnect(russh::Disconnect::ByApplication, "", "en"),
    )
    .await;
    forward_tasks.close();
    while let Ok(forward) = forward_tasks.try_recv() {
        forward.abort();
    }
    tasks.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn agent_forwarding_consent() {
    for kind in [AuthFixtureKind::Password, AuthFixtureKind::PrivateKey] {
        for enabled in [false, true] {
            run_forwarding_consent(ForwardedChannel::Agent, kind, enabled).await;
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn x11_forwarding_consent() {
    for kind in [AuthFixtureKind::Password, AuthFixtureKind::PrivateKey] {
        for enabled in [false, true] {
            run_forwarding_consent(ForwardedChannel::X11, kind, enabled).await;
        }
    }
}

#[tokio::test]
async fn x11_tcp_forwarding_consent() {
    for kind in [AuthFixtureKind::Password, AuthFixtureKind::PrivateKey] {
        for enabled in [false, true] {
            run_forwarding_consent(ForwardedChannel::X11Tcp, kind, enabled).await;
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn x11_unavailable_display_reports_failure_and_closes_channel() {
    for kind in [AuthFixtureKind::Password, AuthFixtureKind::PrivateKey] {
        for enabled in [false, true] {
            run_forwarding_consent(ForwardedChannel::X11Unavailable, kind, enabled).await;
        }
    }
}

#[cfg(unix)]
struct CountingPasswordServer {
    inner: AuthFixtureServer,
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(unix)]
impl ServerHandler for CountingPasswordServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if user != "fixture-user" {
            return Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            });
        }
        match self.inner.auth_password(user, password).await? {
            Auth::Accept => Ok(Auth::Accept),
            // This fixture tests password retries. russh's default rejection
            // removes password from the advertised methods, forbidding retries.
            _ => Ok(Auth::Reject {
                proceed_with_methods: Some(russh::MethodSet::from(
                    &[russh::MethodKind::Password][..],
                )),
                partial_success: false,
            }),
        }
    }
}

#[cfg(unix)]
struct PinnedHostKeyClient(PublicKey);

#[cfg(unix)]
impl client::Handler for PinnedHostKeyClient {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.0.key_data())
    }
}

#[cfg(unix)]
async fn run_manager_auth_fallback(
    saved_password: Option<&str>,
    with_unavailable_keys: bool,
    provided_password: Option<&str>,
) -> (Result<(), crate::ssh::SshError>, usize) {
    use crate::security::{CredentialState, SecretState, VaultSnapshot, VaultSnapshotSecret};
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const IO: Duration = Duration::from_secs(10);
    let directory = tempdir().expect("create manager fallback directory");
    let missing_socket = directory.path().join("missing-agent.sock");
    let key = serde_json::json!({ "user": "fixture-user", "host": "127.0.0.1", "port": 22 });
    let secrets = SecretState::default();
    secrets
        .replace(
            VaultSnapshot {
                config: serde_json::Value::Null,
                secrets: vec![VaultSnapshotSecret {
                    r#type: "password".into(),
                    key: key.as_object().expect("vault key object").clone(),
                    value: saved_password.unwrap_or_default().into(),
                }],
            },
            SecretString::new("vault-passphrase".into()),
            Duration::from_secs(60),
        )
        .expect("initialize in-memory vault");
    let selector = serde_json::json!({ "type": "password", "key": key });
    let mut auth = Vec::new();
    if with_unavailable_keys {
        let malformed_key = directory.path().join("malformed-key");
        fs::write(&malformed_key, b"not a private key").expect("write malformed key fixture");
        for file_ref in [
            directory
                .path()
                .join("missing-key")
                .to_string_lossy()
                .into_owned(),
            directory.path().to_string_lossy().into_owned(),
            "vault://missing-key".into(),
            malformed_key.to_string_lossy().into_owned(),
        ] {
            auth.push(serde_json::json!({ "type": "privateKey", "fileRef": file_ref }));
        }
    }
    auth.push(serde_json::json!({ "type": "agent", "socket": missing_socket }));
    if let Some(password) = provided_password {
        auth.push(serde_json::json!({ "type": "providedPassword", "password": password }));
    }
    if saved_password.is_some() {
        auth.push(serde_json::json!({ "type": "keyboardInteractive" }));
        auth.push(serde_json::json!({
            "type": "password",
            "secretRef": format!("vault-secret://{}", BASE64_STANDARD.encode(selector.to_string())),
        }));
    }

    let host_key = HostKeyAlgorithm::Ed25519.generate();
    let host_public_key = host_key.public_key().clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind manager fallback fixture");
    let port = listener.local_addr().expect("read fixture address").port();
    let request: crate::ssh::SshConnectRequest = serde_json::from_value(serde_json::json!({
        "profileId": "ssh:fixture",
        "host": "127.0.0.1",
        "port": port,
        "username": "fixture-user",
        "auth": auth,
        "terminal": { "term": "xterm-256color", "columns": 80, "rows": 24 },
    }))
    .expect("parse fixture request");
    let mut config = server::Config::default();
    config.keys.push(host_key);
    let attempts = Arc::new(AtomicUsize::new(0));
    let handler = CountingPasswordServer {
        inner: AuthFixtureServer {
            kind: AuthFixtureKind::Password,
            expected: "fixture-secret".into(),
            authorized_public_key: None,
        },
        attempts: Arc::clone(&attempts),
    };
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept manager fallback");
        if let Ok(session) = server::run_stream(Arc::new(config), tcp, handler).await {
            let _ = session.await;
        }
    });

    let credentials = CredentialState::default();
    let authenticator = super::ManagerAuthenticator {
        manager: super::SshManager::new(directory.path().join("known_hosts")),
        app: None,
        request,
        secrets: &secrets,
        credentials: &credentials,
        used_private_key: std::sync::Mutex::new(false),
        resolved_username: std::sync::Mutex::new(None),
    };
    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &SshTarget {
                host: "127.0.0.1".into(),
                port,
                username: "unused-transport-username".into(),
            },
            PinnedHostKeyClient(host_public_key),
            Arc::new(std::sync::Mutex::new(None)),
            &authenticator,
        ),
    )
    .await
    .expect("manager fallback connection timed out");
    let result = match connected {
        Ok(handle) => {
            assert_eq!(authenticator.username().unwrap(), "fixture-user");
            tokio::time::timeout(
                IO,
                handle.disconnect(russh::Disconnect::ByApplication, "", "en"),
            )
            .await
            .expect("disconnect timed out")
            .expect("disconnect failed");
            Ok(())
        }
        Err(error) => Err(error),
    };
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .expect("manager fallback fixture did not stop")
        .expect("manager fallback fixture task missing")
        .expect("manager fallback fixture task panicked");
    (result, attempts.load(Ordering::SeqCst))
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn manager_authenticator_missing_agent_falls_back_to_password() {
    let (result, attempts) = run_manager_auth_fallback(Some("fixture-secret"), false, None).await;
    assert!(result.is_ok(), "missing agent must fall back: {result:?}");
    assert_eq!(attempts, 1);

    let (result, attempts) = run_manager_auth_fallback(None, false, None).await;
    assert!(
        matches!(result, Err(crate::ssh::SshError::AuthenticationExhausted(ref target))
            if target.host == "127.0.0.1" && target.port > 0 && target.username == "fixture-user"),
        "missing agent without password must be rejected: {result:?}"
    );
    assert_eq!(attempts, 0);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn manager_authenticator_unavailable_keys_fall_back_to_password() {
    let (result, attempts) = run_manager_auth_fallback(Some("fixture-secret"), true, None).await;
    assert!(
        result.is_ok(),
        "unavailable keys must fall back: {result:?}"
    );
    assert_eq!(attempts, 1);

    let (result, attempts) = run_manager_auth_fallback(None, true, None).await;
    assert!(
        matches!(result, Err(crate::ssh::SshError::AuthenticationExhausted(ref target))
        if target.host == "127.0.0.1" && target.port > 0 && target.username == "fixture-user")
    );
    assert_eq!(attempts, 0);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn manager_provided_password_authenticates_or_tries_saved_password() {
    for (password, saved, expected_success, expected_attempts) in [
        ("fixture-secret", None, true, 1),
        ("fixture-wrong", None, false, 1),
        ("fixture-wrong", Some("fixture-secret"), true, 2),
        ("fixture-secret", Some("fixture-secret"), true, 1),
        ("fixture-wrong", Some("fixture-wrong"), false, 1),
    ] {
        let (result, attempts) = run_manager_auth_fallback(saved, false, Some(password)).await;
        if expected_success {
            assert!(
                result.is_ok(),
                "configured password authentication failed: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(crate::ssh::SshError::AuthenticationExhausted(ref target))
                if target.host == "127.0.0.1" && target.port > 0 && target.username == "fixture-user")
            );
        }
        assert_eq!(attempts, expected_attempts);
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn manager_keyboard_interactive_empty_challenges_use_real_transport() {
    use crate::security::{CredentialState, SecretState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EmptyChallengeServer {
        responses: Arc<AtomicUsize>,
        accept: bool,
    }
    impl ServerHandler for EmptyChallengeServer {
        type Error = russh::Error;

        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            user: &str,
            _: &str,
            response: Option<Response<'a>>,
        ) -> Result<Auth, Self::Error> {
            assert_eq!(user, "fixture-user");
            if let Some(response) = response {
                assert_eq!(
                    response.count(),
                    0,
                    "empty challenge must receive zero answers"
                );
                if self.responses.fetch_add(1, Ordering::SeqCst) == 1 {
                    return Ok(if self.accept {
                        Auth::Accept
                    } else {
                        Auth::reject()
                    });
                }
            }
            Ok(Auth::Partial {
                name: Cow::Borrowed("No input needed"),
                instructions: Cow::Borrowed("Continue without displaying a prompt"),
                prompts: Cow::Owned(vec![]),
            })
        }
    }

    for accept in [true, false] {
        const IO: Duration = Duration::from_secs(10);
        let directory = tempdir().unwrap();
        let host_key = HostKeyAlgorithm::Ed25519.generate();
        let pinned_key = host_key.public_key().clone();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let responses = Arc::new(AtomicUsize::new(0));
        let handler = EmptyChallengeServer {
            responses: Arc::clone(&responses),
            accept,
        };
        let mut config = server::Config::default();
        config.keys.push(host_key);
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let session = server::run_stream(Arc::new(config), tcp, handler)
                .await
                .unwrap();
            let _ = session.await;
        });
        let secrets = SecretState::default();
        let credentials = CredentialState::default();
        let authenticator = super::ManagerAuthenticator {
            manager: super::SshManager::new(directory.path().join("known_hosts")),
            app: None,
            request: serde_json::from_value(serde_json::json!({
                "profileId": "ssh:empty-challenge", "host": "127.0.0.1", "port": port,
                "username": "fixture-user", "auth": [{ "type": "keyboardInteractive" }],
                "terminal": { "term": "xterm-256color", "columns": 80, "rows": 24 },
            }))
            .unwrap(),
            secrets: &secrets,
            credentials: &credentials,
            used_private_key: std::sync::Mutex::new(false),
            resolved_username: std::sync::Mutex::new(None),
        };
        let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
        let connected = tokio::time::timeout(
            IO,
            engine.connect_with_handler(
                &SshTarget {
                    host: "127.0.0.1".into(),
                    port,
                    username: "fixture-user".into(),
                },
                PinnedHostKeyClient(pinned_key),
                Arc::new(std::sync::Mutex::new(None)),
                &authenticator,
            ),
        )
        .await
        .expect("empty challenge must not wait for user input");
        if accept {
            let handle = connected.expect("empty challenges should authenticate without an app");
            tokio::time::timeout(
                IO,
                handle.disconnect(russh::Disconnect::ByApplication, "", "en"),
            )
            .await
            .unwrap()
            .unwrap();
        } else {
            assert!(
                matches!(connected, Err(crate::ssh::SshError::AuthenticationExhausted(ref target))
                if target.host == "127.0.0.1" && target.port == port && target.username == "fixture-user")
            );
        }
        assert_eq!(responses.load(Ordering::SeqCst), 2);
        tokio::time::timeout(IO, tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_dismissal_skips_wire_reply_and_uses_password_candidate() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DismissalServer(Arc<AtomicUsize>);
    impl ServerHandler for DismissalServer {
        type Error = russh::Error;

        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            _: &str,
            _: &str,
            response: Option<Response<'a>>,
        ) -> Result<Auth, Self::Error> {
            if let Some(response) = response {
                self.0.fetch_add(1, Ordering::SeqCst);
                assert_eq!(response.count(), 0);
                return Ok(Auth::reject());
            }
            Ok(Auth::Partial {
                name: Cow::Borrowed("fixture"),
                instructions: Cow::Borrowed("dismiss this prompt"),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Code: "), false)]),
            })
        }

        async fn auth_password(&mut self, _: &str, password: &str) -> Result<Auth, Self::Error> {
            Ok(if password == "saved-secret" {
                Auth::Accept
            } else {
                Auth::reject()
            })
        }
    }

    struct CancelThenPassword;
    #[async_trait::async_trait]
    impl SshAuthenticator for CancelThenPassword {
        async fn authenticate(
            &self,
            context: &mut dyn SshAuthContext,
            username: &str,
            _: &[crate::ssh::AuthMethodRef],
        ) -> Result<bool, crate::ssh::SshError> {
            let dismissed =
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Ok(vec![])
                })
                .await?;
            assert!(!dismissed, "dismissal must skip this candidate");
            context
                .authenticate_password(username, &SecretString::new("saved-secret".into()))
                .await
        }
    }

    const IO: Duration = Duration::from_secs(10);
    let host_key = HostKeyAlgorithm::Ed25519.generate();
    let pinned_key = host_key.public_key().clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let response_count = Arc::new(AtomicUsize::new(0));
    let handler = DismissalServer(Arc::clone(&response_count));
    let mut config = server::Config::default();
    config.keys.push(host_key);
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let session = server::run_stream(Arc::new(config), tcp, handler)
            .await
            .unwrap();
        let _ = session.await;
    });

    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &SshTarget {
                host: "127.0.0.1".into(),
                port,
                username: "fixture-user".into(),
            },
            PinnedHostKeyClient(pinned_key),
            Arc::new(std::sync::Mutex::new(None)),
            &CancelThenPassword,
        ),
    )
    .await
    .expect("wire fixture must finish within timeout")
    .expect("saved password should authenticate after dismissal");
    assert_eq!(
        response_count.load(Ordering::SeqCst),
        0,
        "dismissed nonempty challenge must not send a keyboard-interactive reply"
    );
    tokio::time::timeout(
        IO,
        connected.disconnect(russh::Disconnect::ByApplication, "test complete", "en"),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum PendingChallengeCase {
    SecondRoundThenPassword,
    NewKeyboardInteractiveCandidate,
    PrivateKeyCandidate,
}

#[cfg(unix)]
#[derive(Default)]
struct PendingChallengeObservations {
    keyboard_interactive_starts: std::sync::atomic::AtomicUsize,
    keyboard_interactive_answers: std::sync::Mutex<Vec<Vec<String>>>,
    password_requests: std::sync::atomic::AtomicUsize,
    key_offers: std::sync::atomic::AtomicUsize,
    key_requests: std::sync::atomic::AtomicUsize,
    matched_key: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
struct PendingChallengeServer {
    case: PendingChallengeCase,
    observations: Arc<PendingChallengeObservations>,
    authorized_public_key: Option<String>,
}

#[cfg(unix)]
impl Server for PendingChallengeServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self {
        Self {
            case: self.case,
            observations: Arc::clone(&self.observations),
            authorized_public_key: self.authorized_public_key.clone(),
        }
    }
}

#[cfg(unix)]
fn read_keyboard_interactive_answers(response: Response<'_>) -> Vec<String> {
    response
        .map(|answer| String::from_utf8_lossy(&answer).into_owned())
        .collect()
}

#[cfg(unix)]
fn pending_challenge(name: &'static str) -> Auth {
    Auth::Partial {
        name: Cow::Borrowed(name),
        instructions: Cow::Borrowed("fixture instruction"),
        prompts: Cow::Owned(vec![(Cow::Borrowed("Fixture answer: "), false)]),
    }
}

#[cfg(unix)]
impl ServerHandler for PendingChallengeServer {
    type Error = russh::Error;

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        assert_eq!(user, "fixture-user");
        match response {
            None => {
                let start = self
                    .observations
                    .keyboard_interactive_starts
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(match (self.case, start) {
                    (PendingChallengeCase::SecondRoundThenPassword, 1) => {
                        pending_challenge("second round")
                    }
                    _ => pending_challenge("first round"),
                })
            }
            Some(response) => {
                let answers = read_keyboard_interactive_answers(response);
                self.observations
                    .keyboard_interactive_answers
                    .lock()
                    .unwrap()
                    .push(answers.clone());
                Ok(match self.case {
                    PendingChallengeCase::SecondRoundThenPassword
                        if answers == ["first-answer"] =>
                    {
                        pending_challenge("second round")
                    }
                    PendingChallengeCase::NewKeyboardInteractiveCandidate
                        if answers == [""]
                            && self
                                .observations
                                .keyboard_interactive_starts
                                .load(std::sync::atomic::Ordering::SeqCst)
                                == 2 =>
                    {
                        Auth::Accept
                    }
                    _ => Auth::reject(),
                })
            }
        }
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        assert_eq!(user, "fixture-user");
        self.observations
            .password_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(if password == "saved-secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        assert_eq!(user, "fixture-user");
        self.observations
            .key_offers
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let matches =
            public_key.to_openssh().ok().as_deref() == self.authorized_public_key.as_deref();
        if matches {
            self.observations
                .matched_key
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        assert_eq!(user, "fixture-user");
        self.observations
            .key_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let matches =
            public_key.to_openssh().ok().as_deref() == self.authorized_public_key.as_deref();
        if matches {
            self.observations
                .matched_key
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(Auth::Accept)
        } else {
            Ok(Auth::reject())
        }
    }
}

#[cfg(unix)]
async fn start_pending_challenge_server(
    case: PendingChallengeCase,
    authorized_public_key: Option<PublicKey>,
) -> (
    u16,
    PublicKey,
    Arc<PendingChallengeObservations>,
    tokio::task::JoinSet<()>,
) {
    let host_key = HostKeyAlgorithm::Ed25519.generate();
    let pinned_key = host_key.public_key().clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind loopback SSH fixture");
    let port = listener.local_addr().expect("fixture local address").port();
    let observations = Arc::new(PendingChallengeObservations::default());
    let handler = PendingChallengeServer {
        case,
        observations: Arc::clone(&observations),
        authorized_public_key: authorized_public_key
            .map(|key| key.to_openssh().expect("encode authorized client key")),
    };
    let mut config = server::Config::default();
    config.keys.push(host_key);
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let (tcp, _) = listener
            .accept()
            .await
            .expect("accept loopback SSH fixture");
        let session = server::run_stream(Arc::new(config), tcp, handler)
            .await
            .expect("start loopback SSH fixture session");
        let _ = session.await;
    });
    (port, pinned_key, observations, tasks)
}

#[cfg(unix)]
fn pending_challenge_target(port: u16) -> SshTarget {
    SshTarget {
        host: "127.0.0.1".into(),
        port,
        username: "fixture-user".into(),
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_second_round_dismissal_uses_password_candidate() {
    use std::sync::atomic::Ordering;

    struct CancelSecondRoundThenPassword;
    #[async_trait::async_trait]
    impl SshAuthenticator for CancelSecondRoundThenPassword {
        async fn authenticate(
            &self,
            context: &mut dyn SshAuthContext,
            username: &str,
            _: &[crate::ssh::AuthMethodRef],
        ) -> Result<bool, crate::ssh::SshError> {
            let mut round = 0;
            let accepted = super::authenticate_keyboard_interactive_with(context, username, |_| {
                round += 1;
                async move {
                    Ok(if round == 1 {
                        vec!["first-answer".into()]
                    } else {
                        vec![]
                    })
                }
            })
            .await?;
            assert!(
                !accepted,
                "second prompt dismissal must skip the KI candidate"
            );
            context
                .authenticate_password(username, &SecretString::new("saved-secret".into()))
                .await
        }
    }

    const IO: Duration = Duration::from_secs(10);
    let (port, pinned_key, observations, mut tasks) =
        start_pending_challenge_server(PendingChallengeCase::SecondRoundThenPassword, None).await;
    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &pending_challenge_target(port),
            PinnedHostKeyClient(pinned_key),
            Arc::new(std::sync::Mutex::new(None)),
            &CancelSecondRoundThenPassword,
        ),
    )
    .await
    .expect("second-round cancellation fixture timed out")
    .expect("password candidate should authenticate after second-round dismissal");
    assert_eq!(
        *observations.keyboard_interactive_answers.lock().unwrap(),
        [vec!["first-answer".to_owned()]],
        "peer must receive the first answer and no response for the dismissed second round"
    );
    assert_eq!(
        observations
            .keyboard_interactive_starts
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(observations.password_requests.load(Ordering::SeqCst), 1);
    tokio::time::timeout(
        IO,
        connected.disconnect(russh::Disconnect::ByApplication, "test complete", "en"),
    )
    .await
    .expect("disconnect fixture timed out")
    .expect("disconnect fixture failed");
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .expect("server task timed out")
        .expect("server task ended without a result")
        .expect("server task failed");
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_dismissal_can_start_another_candidate() {
    use std::sync::atomic::Ordering;

    struct CancelThenNewKeyboardInteractiveCandidate;
    #[async_trait::async_trait]
    impl SshAuthenticator for CancelThenNewKeyboardInteractiveCandidate {
        async fn authenticate(
            &self,
            context: &mut dyn SshAuthContext,
            username: &str,
            _: &[crate::ssh::AuthMethodRef],
        ) -> Result<bool, crate::ssh::SshError> {
            let first =
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Ok(vec![])
                })
                .await?;
            assert!(!first, "first KI candidate must be dismissed");
            super::authenticate_keyboard_interactive_with(context, username, |_| async {
                Ok(vec![String::new()])
            })
            .await
        }
    }

    const IO: Duration = Duration::from_secs(10);
    let (port, pinned_key, observations, mut tasks) =
        start_pending_challenge_server(PendingChallengeCase::NewKeyboardInteractiveCandidate, None)
            .await;
    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &pending_challenge_target(port),
            PinnedHostKeyClient(pinned_key),
            Arc::new(std::sync::Mutex::new(None)),
            &CancelThenNewKeyboardInteractiveCandidate,
        ),
    )
    .await
    .expect("new keyboard-interactive candidate fixture timed out")
    .expect("blank response on the second KI candidate should authenticate");
    assert_eq!(
        observations
            .keyboard_interactive_starts
            .load(Ordering::SeqCst),
        2,
        "peer must observe two distinct keyboard-interactive requests"
    );
    assert_eq!(
        *observations.keyboard_interactive_answers.lock().unwrap(),
        [vec![String::new()]],
        "only the second candidate's one blank answer must reach the peer"
    );
    assert_eq!(observations.password_requests.load(Ordering::SeqCst), 0);
    tokio::time::timeout(
        IO,
        connected.disconnect(russh::Disconnect::ByApplication, "test complete", "en"),
    )
    .await
    .expect("disconnect fixture timed out")
    .expect("disconnect fixture failed");
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .expect("server task timed out")
        .expect("server task ended without a result")
        .expect("server task failed");
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_dismissal_uses_private_key_candidate() {
    use std::sync::atomic::Ordering;

    struct CancelThenPrivateKey(Option<super::engine::PrivateKeyMaterial>);
    #[async_trait::async_trait]
    impl SshAuthenticator for CancelThenPrivateKey {
        async fn authenticate(
            &self,
            context: &mut dyn SshAuthContext,
            username: &str,
            _: &[crate::ssh::AuthMethodRef],
        ) -> Result<bool, crate::ssh::SshError> {
            let dismissed =
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Ok(vec![])
                })
                .await?;
            assert!(
                !dismissed,
                "KI candidate must be dismissed before attempting the key"
            );
            context
                .authenticate_private_key(
                    username,
                    self.0.clone().expect("generated client private key"),
                )
                .await
        }
    }

    const IO: Duration = Duration::from_secs(10);
    let (private_key, public_key) = private_key_fixture(false);
    let (port, pinned_key, observations, mut tasks) =
        start_pending_challenge_server(PendingChallengeCase::PrivateKeyCandidate, Some(public_key))
            .await;
    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &pending_challenge_target(port),
            PinnedHostKeyClient(pinned_key),
            Arc::new(std::sync::Mutex::new(None)),
            &CancelThenPrivateKey(Some(private_key)),
        ),
    )
    .await
    .expect("private-key candidate fixture timed out")
    .expect("verified private key should authenticate after KI dismissal");
    assert_eq!(
        observations
            .keyboard_interactive_starts
            .load(Ordering::SeqCst),
        1
    );
    assert!(
        observations
            .keyboard_interactive_answers
            .lock()
            .unwrap()
            .is_empty(),
        "dismissed keyboard-interactive challenge must receive no response"
    );
    assert_eq!(observations.key_offers.load(Ordering::SeqCst), 1);
    assert_eq!(observations.key_requests.load(Ordering::SeqCst), 1);
    assert!(observations.matched_key.load(Ordering::SeqCst));
    assert_eq!(observations.password_requests.load(Ordering::SeqCst), 0);
    tokio::time::timeout(
        IO,
        connected.disconnect(russh::Disconnect::ByApplication, "test complete", "en"),
    )
    .await
    .expect("disconnect fixture timed out")
    .expect("disconnect fixture failed");
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .expect("server task timed out")
        .expect("server task ended without a result")
        .expect("server task failed");
}
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_dismissal_uses_real_ssh_agent_candidate() {
    use std::sync::atomic::Ordering;

    struct CancelThenAgent {
        socket: String,
    }
    #[async_trait::async_trait]
    impl SshAuthenticator for CancelThenAgent {
        async fn authenticate(
            &self,
            context: &mut dyn SshAuthContext,
            username: &str,
            _: &[crate::ssh::AuthMethodRef],
        ) -> Result<bool, crate::ssh::SshError> {
            let dismissed =
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Ok(vec![])
                })
                .await?;
            assert!(
                !dismissed,
                "KI candidate must be dismissed before using the agent"
            );
            context
                .authenticate_agent(username, Some(&self.socket))
                .await
        }
    }

    const IO: Duration = Duration::from_secs(10);
    let agent_output = Command::new("ssh-agent")
        .arg("-s")
        .output()
        .expect("start isolated ssh-agent");
    assert!(agent_output.status.success(), "ssh-agent failed to start");
    let agent_output = String::from_utf8(agent_output.stdout).expect("ssh-agent output is UTF-8");
    let socket = agent_output
        .lines()
        .find_map(|line| line.strip_prefix("SSH_AUTH_SOCK=")?.split(';').next())
        .map(str::to_owned)
        .expect("ssh-agent did not report its socket");
    let pid = agent_output
        .lines()
        .find_map(|line| line.strip_prefix("SSH_AGENT_PID=")?.split(';').next())
        .map(str::to_owned)
        .expect("ssh-agent did not report its PID");
    let _agent = SshAgentGuard {
        pid,
        socket: socket.clone(),
    };

    let directory = tempdir().expect("create isolated ssh-agent key directory");
    let key_path = directory.path().join("fixture_ed25519");
    let generated = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key_path)
        .output()
        .expect("generate synthetic Ed25519 key");
    assert!(
        generated.status.success(),
        "ssh-keygen failed to create fixture key"
    );
    let public_key_text =
        fs::read_to_string(key_path.with_extension("pub")).expect("read synthetic public key");
    let public_key = parse_public_key_base64(
        public_key_text
            .split_whitespace()
            .nth(1)
            .expect("synthetic public key is malformed"),
    )
    .expect("parse synthetic public key");
    let added = Command::new("ssh-add")
        .arg("-q")
        .arg(&key_path)
        .env("SSH_AUTH_SOCK", &socket)
        .output()
        .expect("load synthetic key into isolated ssh-agent");
    assert!(added.status.success(), "ssh-add failed to load fixture key");

    let (port, pinned_key, observations, mut tasks) =
        start_pending_challenge_server(PendingChallengeCase::PrivateKeyCandidate, Some(public_key))
            .await;
    let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
    let connected = tokio::time::timeout(
        IO,
        engine.connect_with_handler(
            &pending_challenge_target(port),
            PinnedHostKeyClient(pinned_key),
            Arc::new(std::sync::Mutex::new(None)),
            &CancelThenAgent { socket },
        ),
    )
    .await
    .expect("ssh-agent candidate fixture timed out")
    .expect("ssh-agent key should authenticate after KI dismissal");
    assert_eq!(
        observations
            .keyboard_interactive_starts
            .load(Ordering::SeqCst),
        1
    );
    assert!(observations
        .keyboard_interactive_answers
        .lock()
        .unwrap()
        .is_empty());
    assert_eq!(observations.key_offers.load(Ordering::SeqCst), 1);
    assert_eq!(observations.key_requests.load(Ordering::SeqCst), 1);
    assert!(observations.matched_key.load(Ordering::SeqCst));
    assert_eq!(observations.password_requests.load(Ordering::SeqCst), 0);
    tokio::time::timeout(
        IO,
        connected.disconnect(russh::Disconnect::ByApplication, "test complete", "en"),
    )
    .await
    .expect("disconnect fixture timed out")
    .expect("disconnect fixture failed");
    tokio::time::timeout(IO, tasks.join_next())
        .await
        .expect("server task timed out")
        .expect("server task ended without a result")
        .expect("server task failed");
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum TerminalPromptOutcome {
    Closed,
    Timeout,
    CancelWithoutCandidate,
    CancelThenWrongPassword,
}

#[cfg(unix)]
struct TerminalPromptAuthenticator(TerminalPromptOutcome);

#[cfg(unix)]
#[async_trait::async_trait]
impl SshAuthenticator for TerminalPromptAuthenticator {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        _: &[crate::ssh::AuthMethodRef],
    ) -> Result<bool, crate::ssh::SshError> {
        use TerminalPromptOutcome::*;
        match self.0 {
            Closed => {
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Err(crate::ssh::SshError::Closed)
                })
                .await
            }
            Timeout => {
                super::authenticate_keyboard_interactive_with(context, username, |_| async {
                    Err(crate::ssh::SshError::Timeout)
                })
                .await
            }
            CancelWithoutCandidate | CancelThenWrongPassword => {
                let dismissed =
                    super::authenticate_keyboard_interactive_with(context, username, |_| async {
                        Ok(vec![])
                    })
                    .await?;
                assert!(!dismissed, "empty response must dismiss this candidate");
                match self.0 {
                    CancelWithoutCandidate => Ok(false),
                    CancelThenWrongPassword => {
                        context
                            .authenticate_password(
                                username,
                                &SecretString::new("wrong-fixture-password".into()),
                            )
                            .await
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SSH authentication fixture; run yarn test:ssh-auth-integration"]
async fn keyboard_interactive_terminal_outcomes_close_without_stray_auth_requests() {
    use std::sync::atomic::Ordering;

    use TerminalPromptOutcome::*;
    const IO: Duration = Duration::from_secs(10);
    for (name, outcome, expected_password_requests, expected_error) in [
        ("Closed", Closed, 0, "closed"),
        ("Timeout", Timeout, 0, "timeout"),
        (
            "cancel without candidate",
            CancelWithoutCandidate,
            0,
            "authenticationRejected",
        ),
        (
            "cancel then wrong password",
            CancelThenWrongPassword,
            1,
            "authenticationRejected",
        ),
    ] {
        let (port, pinned_key, observations, mut tasks) =
            start_pending_challenge_server(PendingChallengeCase::PrivateKeyCandidate, None).await;
        let engine = super::engine::RusshEngine::new(client::Config::default(), IO);
        let result = tokio::time::timeout(
            IO,
            engine.connect_with_handler(
                &pending_challenge_target(port),
                PinnedHostKeyClient(pinned_key),
                Arc::new(std::sync::Mutex::new(None)),
                &TerminalPromptAuthenticator(outcome),
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} outcome exceeded bounded connect timeout"));
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{name} unexpectedly authenticated"),
        };
        assert_eq!(
            error.code(),
            expected_error,
            "{name} unexpected engine error: {error:?}"
        );
        assert_eq!(
            observations
                .keyboard_interactive_starts
                .load(Ordering::SeqCst),
            1,
            "{name} must have exactly one peer KI start"
        );
        assert!(
            observations
                .keyboard_interactive_answers
                .lock()
                .unwrap()
                .is_empty(),
            "{name} must not send a keyboard-interactive answer"
        );
        assert_eq!(
            observations.password_requests.load(Ordering::SeqCst),
            expected_password_requests,
            "{name} peer password request count"
        );
        assert_eq!(observations.key_offers.load(Ordering::SeqCst), 0);
        assert_eq!(observations.key_requests.load(Ordering::SeqCst), 0);
        assert!(!observations.matched_key.load(Ordering::SeqCst));
        tokio::time::timeout(IO, tasks.join_next())
            .await
            .unwrap_or_else(|_| panic!("{name} server task did not stop"))
            .expect("server task ended without a result")
            .expect("server task failed");
    }
}
