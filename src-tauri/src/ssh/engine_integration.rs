use std::{borrow::Cow, sync::Arc, time::Duration};

use russh::{
    client,
    keys::{PrivateKey, PublicKey},
    server::{self, Auth, Handler as ServerHandler, Response, Server},
    SshId,
};
use secrecy::{ExposeSecret, SecretString};

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
        if !matches!(self.kind, AuthFixtureKind::PrivateKey) {
            return false;
        }
        let Ok(public_key) = public_key.to_openssh() else {
            return false;
        };
        self.authorized_public_key.as_deref() == Some(public_key.as_str())
    }
}

struct AuthFixtureAuthenticator {
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
        run_russh_auth_fixture(AuthFixtureKind::Password, host_key.clone(), None, None).await;
        run_russh_auth_fixture(
            AuthFixtureKind::KeyboardInteractive,
            host_key.clone(),
            None,
            None,
        )
        .await;
        for encrypted in [false, true] {
            let (private_key, public_key) = private_key_fixture(encrypted);
            run_russh_auth_fixture(
                AuthFixtureKind::PrivateKey,
                host_key.clone(),
                Some(private_key),
                Some(public_key),
            )
            .await;
        }
    }
}
