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
