use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{client, server, Channel, ChannelId, ChannelMsg, CryptoVec};

use super::{start_shell_channel, SshError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Accept,
    Reject,
    Silent,
    Eof,
    Close,
}

struct Fixture {
    optional: Reply,
    shell: Reply,
    requests: Arc<Mutex<Vec<&'static str>>>,
}

impl Fixture {
    fn request(
        &self,
        name: &'static str,
        ch: ChannelId,
        reply: Reply,
        s: &mut server::Session,
    ) -> Result<(), russh::Error> {
        self.requests.lock().unwrap().push(name);
        s.data(ch, CryptoVec::from_slice(name.as_bytes()))?;
        // The SSH server respects want_reply, so these policies emit no optional
        // channel replies unless a regression starts requesting them again.
        match reply {
            Reply::Accept => s.channel_success(ch),
            Reply::Reject => s.channel_failure(ch),
            Reply::Silent => Ok(()),
            Reply::Eof => s.eof(ch),
            Reply::Close => s.close(ch),
        }
    }
}

impl server::Handler for Fixture {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn x11_request(
        &mut self,
        ch: ChannelId,
        single: bool,
        protocol: &str,
        cookie: &str,
        screen: u32,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        assert!(!single);
        assert_eq!(protocol, "MIT-MAGIC-COOKIE-1");
        assert_eq!(screen, 0);
        assert_eq!(cookie.len(), 32);
        assert!(cookie
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        self.request("x11", ch, self.optional, s)
    }

    async fn agent_request(
        &mut self,
        ch: ChannelId,
        s: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.request("agent", ch, self.optional, s)?;
        Ok(self.optional == Reply::Accept)
    }

    async fn shell_request(
        &mut self,
        ch: ChannelId,
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.request("shell", ch, self.shell, s)
    }

    async fn data(
        &mut self,
        ch: ChannelId,
        data: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        s.data(ch, CryptoVec::from_slice(data))
    }
}

struct PinnedClient(russh::keys::PublicKey);

impl client::Handler for PinnedClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.0.key_data())
    }
}

async fn run_case(x11: bool, agent: bool, optional: Reply, shell: Reply) {
    const IO: Duration = Duration::from_secs(5);
    let key =
        russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
            .unwrap();
    let public_key = key.public_key().clone();
    let mut config = server::Config::default();
    config.keys.push(key);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let fixture = Fixture {
        optional,
        shell,
        requests: Arc::clone(&requests),
    };
    // JoinSet aborts the fixture even when a regression panics or times out.
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let session = server::run_stream(Arc::new(config), tcp, fixture)
            .await
            .unwrap();
        let _ = session.await;
    });
    tokio::time::timeout(IO, async {
        let mut connection = client::connect(
            Arc::new(client::Config::default()),
            address,
            PinnedClient(public_key),
        )
        .await
        .unwrap();
        assert!(connection
            .authenticate_none("fixture-user")
            .await
            .unwrap()
            .success());
        let mut channel = connection.channel_open_session().await.unwrap();
        let result = start_shell_channel(&mut channel, x11, agent).await;
        let mut expected = Vec::new();
        if x11 {
            expected.push("x11");
        }
        if agent {
            expected.push("agent");
        }
        expected.push("shell");
        assert_eq!(
            *requests.lock().unwrap(),
            expected,
            "optional={optional:?}, shell={shell:?}"
        );
        if shell == Reply::Accept {
            let pending = result.expect("optional forwarding rejection must not prevent a shell");
            let data: Vec<u8> = pending
                .into_iter()
                .flat_map(|message| match message {
                    ChannelMsg::Data { data } => data.to_vec(),
                    other => panic!("unexpected pending message: {other:?}"),
                })
                .collect();
            assert_eq!(
                data,
                expected.concat().as_bytes(),
                "preserve output before shell confirmation"
            );
            channel
                .data(&b"terminal input after forwarding"[..])
                .await
                .unwrap();
            match channel.wait().await.unwrap() {
                ChannelMsg::Data { data } => {
                    assert_eq!(&data[..], b"terminal input after forwarding")
                }
                other => panic!("expected terminal echo, got {other:?}"),
            }
        } else {
            assert!(
                matches!(result, Err(SshError::ChannelOpen)),
                "failed or closed shell must not succeed: {result:?}"
            );
        }
        connection
            .disconnect(russh::Disconnect::ByApplication, "fixture complete", "en")
            .await
            .unwrap();
        tasks.join_next().await.unwrap().unwrap();
    })
    .await
    .expect("shell setup and fixture cleanup must finish without waiting for optional replies");
}

#[tokio::test]
async fn optional_forwarding_preserves_order_output_and_usable_shell() {
    for optional in [Reply::Accept, Reply::Reject, Reply::Silent] {
        for (x11, agent) in [(false, false), (true, false), (false, true), (true, true)] {
            run_case(x11, agent, optional, Reply::Accept).await;
        }
    }
}

#[tokio::test]
async fn shell_failure_or_close_is_not_accepted_after_optional_requests() {
    for optional in [Reply::Accept, Reply::Reject, Reply::Silent] {
        for shell in [Reply::Reject, Reply::Eof, Reply::Close] {
            run_case(true, true, optional, shell).await;
        }
    }
}
