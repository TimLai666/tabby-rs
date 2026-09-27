use std::{sync::Arc, time::Duration};

use russh::{server, Channel, ChannelId, CryptoVec};
use tokio::sync::oneshot;

use super::*;

struct Server(Option<oneshot::Sender<(server::Handle, ChannelId)>>);

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        if let Some(sender) = self.0.take() {
            sender.send((session.handle(), channel.id())).ok();
        }
        Ok(true)
    }
}

struct Client(russh::keys::PublicKey);

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(self.0.key_data() == key.key_data())
    }
}

struct Fixture {
    handle: client::Handle<Client>,
    reader: ChannelReadHalf,
    _writer: russh::ChannelWriteHalf<client::Msg>,
    pending: VecDeque<ChannelMsg>,
    controls: mpsc::Receiver<SshControl>,
    sender: mpsc::Sender<SshControl>,
    channel_closed: bool,
    server: server::Handle,
    id: ChannelId,
    tasks: tokio::task::JoinSet<()>,
}

impl Fixture {
    async fn new() -> Self {
        let key = russh::keys::PrivateKey::random(
            &mut rand::rngs::OsRng,
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = key.public_key().clone();
        let mut config = server::Config::default();
        config.keys.push(key);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (server_tx, server_rx) = oneshot::channel();
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let session = server::run_stream(Arc::new(config), tcp, Server(Some(server_tx)))
                .await
                .unwrap();
            let _ = session.await;
        });
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            address,
            Client(public_key),
        )
        .await
        .unwrap();
        assert!(handle.authenticate_none("fixture").await.unwrap().success());
        let channel = handle.channel_open_session().await.unwrap();
        let (reader, writer) = channel.split();
        let (server, id) = server_rx.await.unwrap();
        let (sender, controls) = mpsc::channel(4);
        Self {
            handle,
            reader,
            _writer: writer,
            pending: VecDeque::new(),
            controls,
            sender,
            channel_closed: false,
            server,
            id,
            tasks,
        }
    }

    async fn next(&mut self) -> Option<ShellEvent> {
        next_shell_event(
            &mut self.reader,
            &mut self.pending,
            &mut self.controls,
            &mut self.handle,
            &mut self.channel_closed,
        )
        .await
    }

    async fn expect_data(&mut self, expected: &[u8]) {
        match self.next().await {
            Some(ShellEvent::Channel(ChannelMsg::Data { data })) => assert_eq!(&data[..], expected),
            _ => panic!("expected ordered channel output"),
        }
    }

    async fn finish(mut self) {
        let _ = self
            .handle
            .disconnect(russh::Disconnect::ByApplication, "done", "en")
            .await;
        self.tasks.join_next().await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn eof_ends_shell_after_buffered_and_remote_output() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut f = Fixture::new().await;
        f.pending.push_back(ChannelMsg::Data {
            data: CryptoVec::from_slice(b"buffered"),
        });
        f.server
            .data(f.id, CryptoVec::from_slice(b"remote"))
            .await
            .unwrap();
        f.server.eof(f.id).await.unwrap();
        f.expect_data(b"buffered").await;
        f.expect_data(b"remote").await;
        assert!(
            f.next().await.is_none(),
            "EOF must end the shell without waiting for CLOSE"
        );
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn channel_close_keeps_transport_and_controls_available() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut f = Fixture::new().await;
        f.server
            .data(f.id, CryptoVec::from_slice(b"before close"))
            .await
            .unwrap();
        f.server.close(f.id).await.unwrap();
        f.expect_data(b"before close").await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), f.next())
                .await
                .is_err(),
            "CLOSE alone must not end the shell"
        );
        assert!(f.channel_closed);
        let another = f.handle.channel_open_session().await.unwrap();
        another.close().await.unwrap();
        // More than the peer's default window must never enter the closed writer.
        let (write_tx, mut write_rx) = oneshot::channel();
        f.sender
            .send(SshControl::Write(vec![b'x'; 8 * 1024 * 1024], write_tx))
            .await
            .unwrap();
        let (resize_tx, mut resize_rx) = oneshot::channel();
        f.sender
            .send(SshControl::Resize(
                super::super::model::SshResizeRequest {
                    id: "fixture".into(),
                    columns: 80,
                    rows: 24,
                    pixel_width: None,
                    pixel_height: None,
                },
                resize_tx,
            ))
            .await
            .unwrap();
        let (tx, _rx) = oneshot::channel();
        f.sender.send(SshControl::Close(tx)).await.unwrap();
        assert!(matches!(
            f.next().await,
            Some(ShellEvent::Control(SshControl::Close(_)))
        ));
        assert!(matches!(
            write_rx.try_recv(),
            Ok(Err(super::super::SshError::Closed))
        ));
        assert!(matches!(
            resize_rx.try_recv(),
            Ok(Err(super::super::SshError::Closed))
        ));
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_close_precedes_ready_writes() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut f = Fixture::new().await;
        for _ in 0..32 {
            f.channel_closed = false;
            f.pending.push_back(ChannelMsg::Close);
            let (write_tx, mut write_rx) = oneshot::channel();
            f.sender
                .send(SshControl::Write(vec![0; 8 * 1024 * 1024], write_tx))
                .await
                .unwrap();
            let (close_tx, _close_rx) = oneshot::channel();
            f.sender.send(SshControl::Close(close_tx)).await.unwrap();
            assert!(matches!(
                f.next().await,
                Some(ShellEvent::Control(SshControl::Close(_)))
            ));
            assert!(matches!(
                write_rx.try_recv(),
                Ok(Err(super::super::SshError::Closed))
            ));
        }
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn status_and_signal_preserve_later_output_and_controls() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut f = Fixture::new().await;
        f.server.exit_status_request(f.id, 42).await.unwrap();
        f.server.exit_signal_request(f.id, russh::Sig::TERM, false, "fixture".into(), "en".into()).await.unwrap();
        f.server.data(f.id, CryptoVec::from_slice(b"after status and signal")).await.unwrap();
        f.expect_data(b"after status and signal").await;
        let (tx, _rx) = oneshot::channel();
        f.sender.send(SshControl::Write(b"input".to_vec(), tx)).await.unwrap();
        assert!(matches!(f.next().await, Some(ShellEvent::Control(SshControl::Write(data, _))) if data == b"input"));
        f.finish().await;
    }).await.unwrap();
}

#[tokio::test]
async fn disconnect_ends_shell_with_or_without_prior_channel_close() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for close_first in [false, true] {
            let mut f = Fixture::new().await;
            f.server
                .data(f.id, CryptoVec::from_slice(b"last output"))
                .await
                .unwrap();
            if close_first {
                f.server.close(f.id).await.unwrap();
            }
            f.server
                .disconnect(
                    russh::Disconnect::ByApplication,
                    "fixture done".into(),
                    "en".into(),
                )
                .await
                .unwrap();
            f.expect_data(b"last output").await;
            assert!(
                f.next().await.is_none(),
                "transport disconnection must end the shell"
            );
            f.finish().await;
        }
    })
    .await
    .unwrap();
}
