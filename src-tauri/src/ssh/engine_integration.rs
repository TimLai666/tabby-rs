use std::{borrow::Cow, sync::Arc, time::Duration};

use russh::{
    client,
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
}

struct AuthFixtureServer {
    kind: AuthFixtureKind,
    expected: String,
}

impl Server for AuthFixtureServer {
    type Handler = Self;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self {
        Self {
            kind: self.kind,
            expected: self.expected.clone(),
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

struct AuthFixtureAuthenticator {
    kind: AuthFixtureKind,
    expected: SecretString,
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

async fn run_russh_auth_fixture(kind: AuthFixtureKind) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind russh auth fixture");
    let port = listener
        .local_addr()
        .expect("read russh fixture address")
        .port();
    let mut config = server::Config::default();
    config.server_id = SshId::Standard("SSH-2.0-tabby-rs-auth-fixture".into());
    config.keys.push(
        russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
            .expect("generate russh fixture host key"),
    );
    let config = Arc::new(config);
    let expected = "fixture-secret";
    let mut server = AuthFixtureServer {
        kind,
        expected: expected.into(),
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
    let connection = engine
        .connect(
            SshTarget {
                host: "127.0.0.1".into(),
                port,
                username: "fixture-user".into(),
            },
            Arc::new(FixtureHostKeyVerifier),
            Arc::new(AuthFixtureAuthenticator {
                kind,
                expected: SecretString::new(expected.into()),
            }),
        )
        .await
        .expect("russh auth fixture connection failed");
    connection
        .disconnect()
        .await
        .expect("disconnect russh auth fixture");
    server_task.await.expect("join russh auth fixture");
}

#[tokio::test]
#[ignore = "requires SSH integration fixture; run yarn test:ssh-integration"]
async fn runs_real_password_and_keyboard_interactive_auth_matrix() {
    run_russh_auth_fixture(AuthFixtureKind::Password).await;
    run_russh_auth_fixture(AuthFixtureKind::KeyboardInteractive).await;
}
