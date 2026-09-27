use std::{collections::BTreeMap, sync::Arc, time::Duration};

use russh::{server, Channel, ChannelId, CryptoVec, Pty};

use super::{engine::*, AuthMethodRef, SshError};

struct Auth;

#[async_trait::async_trait]
impl SshAuthenticator for Auth {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        _: &[AuthMethodRef],
    ) -> Result<bool, SshError> {
        context.authenticate_none(username).await
    }
}

struct PinnedKey(String);

#[async_trait::async_trait]
impl HostKeyVerifier for PinnedKey {
    async fn verify(&self, _: &str, _: u16, key: &SshHostKey) -> Result<bool, SshError> {
        Ok(key.public_key_openssh == self.0)
    }
}

struct Fixture {
    reply: &'static str,
    environment: bool,
}

impl server::Handler for Fixture {
    type Error = russh::Error;

    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        columns: u32,
        rows: u32,
        pixel_width: u32,
        pixel_height: u32,
        modes: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        assert_eq!(term, "xterm-256color");
        assert_eq!(
            (columns, rows, pixel_width, pixel_height),
            (91, 37, 910, 740)
        );
        assert!(modes.is_empty());
        session.data(channel, CryptoVec::from_slice(b"pty"))?;
        // russh sends replies only if the client requested one.
        match self.reply {
            "accept" => session.channel_success(channel),
            "reject" => session.channel_failure(channel),
            "silent" => Ok(()),
            _ => unreachable!(),
        }
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        value: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        assert!(self.environment);
        assert_eq!((name, value), ("FIXTURE", "value"));
        session.data(channel, CryptoVec::from_slice(b"env"))?;
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, CryptoVec::from_slice(b"shell"))?;
        session.channel_success(channel)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, CryptoVec::from_slice(data))
    }
}

async fn run_case(reply: &'static str, environment: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let key = russh::keys::PrivateKey::random(
            &mut rand::rngs::OsRng,
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let verifier = Arc::new(PinnedKey(key.public_key().to_openssh().unwrap()));
        let mut config = server::Config::default();
        config.keys.push(key);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let session = server::run_stream(Arc::new(config), tcp, Fixture { reply, environment })
                .await
                .unwrap();
            let _ = session.await;
        });
        let connection = RusshEngine::new(Default::default(), Duration::from_secs(2))
            .connect(
                SshTarget {
                    host: address.ip().to_string(),
                    port: address.port(),
                    username: "fixture".into(),
                },
                verifier,
                Arc::new(Auth),
            )
            .await
            .unwrap();
        let channel = connection
            .open_shell(ShellChannelRequest {
                term: "xterm-256color".into(),
                columns: 91,
                rows: 37,
                pixel_width: 910,
                pixel_height: 740,
                environment: if environment {
                    BTreeMap::from([("FIXTURE".into(), "value".into())])
                } else {
                    BTreeMap::new()
                },
            })
            .await
            .expect("PTY reply policy must not prevent shell setup");
        channel.write(b"input").await.unwrap();
        let expected = if environment {
            b"ptyenvshellinput".as_slice()
        } else {
            b"ptyshellinput".as_slice()
        };
        let mut output = Vec::new();
        while output.len() < expected.len() {
            match channel.read().await.unwrap() {
                Some(SshChannelMessage::Data(data)) => output.extend_from_slice(&data),
                message => panic!("expected diagnostic and input bytes, got {message:?}"),
            }
        }
        assert_eq!(output, expected);
        channel.close().await.unwrap();
        connection.disconnect().await.unwrap();
        tasks.join_next().await.unwrap().unwrap();
    })
    .await
    .expect("PTY reply must not stall shell setup or cleanup");
}

#[tokio::test]
async fn pty_accept_preserves_diagnostics_and_input() {
    for environment in [false, true] {
        run_case("accept", environment).await;
    }
}

#[tokio::test]
async fn pty_rejection_preserves_diagnostics_and_input() {
    for environment in [false, true] {
        run_case("reject", environment).await;
    }
}

#[tokio::test]
async fn pty_silence_preserves_diagnostics_and_input() {
    for environment in [false, true] {
        run_case("silent", environment).await;
    }
}
