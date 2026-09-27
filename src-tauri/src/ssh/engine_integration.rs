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
        if matches!(self.kind, AuthFixtureKind::Password) && password == self.expected {
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

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ForwardedChannel {
    Agent,
    X11,
}

#[cfg(unix)]
struct ForwardingConsentClient {
    host_key: PublicKey,
    socket: String,
    enabled: bool,
    forwards: tokio::sync::mpsc::UnboundedSender<tokio::task::JoinHandle<Result<(), russh::Error>>>,
}

#[cfg(unix)]
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
        let _ = self.forwards.send(tokio::spawn(super::forward_x11_channel(
            channel,
            Some(self.socket.clone()),
            self.enabled,
        )));
        async { Ok(()) }
    }
}

#[cfg(unix)]
async fn run_forwarding_consent(forwarded: ForwardedChannel, kind: AuthFixtureKind, enabled: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const IO: Duration = Duration::from_secs(5);
    const REQUEST_IDENTITIES: [u8; 5] = [0, 0, 0, 1, 11];
    const IDENTITIES_ANSWER: [u8; 9] = [0, 0, 0, 5, 12, 0, 0, 0, 0];

    let mut tasks = tokio::task::JoinSet::new();
    let directory = tempdir().expect("create agent consent directory");
    let socket = directory.path().join("forward.sock");
    let agent = tokio::net::UnixListener::bind(&socket).expect("bind custom local socket");
    let accepted = Arc::new(AtomicUsize::new(0));
    let agent_accepted = Arc::clone(&accepted);
    tasks.spawn(async move {
        let (mut stream, _) = agent.accept().await.expect("accept custom agent socket");
        agent_accepted.fetch_add(1, Ordering::SeqCst);
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
    });

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
        socket: socket.to_string_lossy().into_owned(),
        enabled,
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
        ForwardedChannel::Agent => {
            tokio::time::timeout(IO, server_handle.channel_open_agent()).await
        }
        ForwardedChannel::X11 => {
            tokio::time::timeout(IO, server_handle.channel_open_x11("127.0.0.1", 0)).await
        }
    }
    .expect("open unsolicited channel timed out")
    .expect("open unsolicited channel failed");
    let mut stream = channel.into_stream();
    let written = stream.write_all(&REQUEST_IDENTITIES).await;
    if enabled {
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
            .expect("disabled forwarding must close the channel, not time out");
        assert!(
            matches!(read, Ok(0) | Err(_)),
            "disabled forwarding reached the local socket: {:?}",
            read.map(|count| buffer[..count].to_vec())
        );
        assert_eq!(accepted.load(Ordering::SeqCst), 0);
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
        self.inner.auth_password(user, password).await
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
async fn run_manager_missing_agent(with_password: bool) -> (Result<(), crate::ssh::SshError>, usize) {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use crate::security::{CredentialState, SecretState, VaultSnapshot, VaultSnapshotSecret};
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
                    value: "fixture-secret".into(),
                }],
            },
            SecretString::new("vault-passphrase".into()),
            Duration::from_secs(60),
        )
        .expect("initialize in-memory vault");
    let selector = serde_json::json!({ "type": "password", "key": key });
    let mut auth = vec![serde_json::json!({ "type": "agent", "socket": missing_socket })];
    if with_password {
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
            PinnedHostKeyClient(host_public_key),
            Arc::new(std::sync::Mutex::new(None)),
            &authenticator,
        ),
    )
    .await
    .expect("manager fallback connection timed out");
    let result = match connected {
        Ok(handle) => {
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
    let (result, attempts) = run_manager_missing_agent(true).await;
    assert!(result.is_ok(), "missing agent must fall back: {result:?}");
    assert_eq!(attempts, 1);

    let (result, attempts) = run_manager_missing_agent(false).await;
    assert!(
        matches!(result, Err(crate::ssh::SshError::AuthenticationRejected)),
        "missing agent without password must be rejected: {result:?}"
    );
    assert_eq!(attempts, 0);
}
