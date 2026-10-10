use std::{sync::Arc, time::Duration};

use russh::{server, Channel, ChannelId, CryptoVec};
use tokio::sync::oneshot;

use super::*;

struct Server(
    Option<oneshot::Sender<(server::Handle, ChannelId)>>,
    mpsc::Sender<ChannelId>,
);

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

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, CryptoVec::from_slice(data))
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.1.send(channel).await.ok();
        Ok(())
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
    _writer: Arc<russh::ChannelWriteHalf<client::Msg>>,
    pending: VecDeque<ChannelMsg>,
    controls: mpsc::Receiver<SshControl>,
    sender: mpsc::Sender<SshControl>,
    channel_closed: bool,
    server: server::Handle,
    id: ChannelId,
    tasks: tokio::task::JoinSet<()>,
    closed: mpsc::Receiver<ChannelId>,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_window(server::Config::default().window_size).await
    }

    async fn with_window(window_size: u32) -> Self {
        let key = russh::keys::PrivateKey::random(
            &mut rand::rngs::OsRng,
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = key.public_key().clone();
        let mut config = server::Config {
            window_size,
            ..Default::default()
        };
        config.keys.push(key);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (server_tx, server_rx) = oneshot::channel();
        let (closed_tx, closed) = mpsc::channel(4);
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let session =
                server::run_stream(Arc::new(config), tcp, Server(Some(server_tx), closed_tx))
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
            _writer: Arc::new(writer),
            pending: VecDeque::new(),
            controls,
            sender,
            channel_closed: false,
            server,
            id,
            tasks,
            closed,
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

    async fn expect_end(&mut self) {
        while let Some(event) = self.next().await {
            assert!(matches!(event, ShellEvent::ChannelClosed));
        }
    }
}

#[tokio::test]
async fn stalled_shell_input_does_not_block_output_or_teardown() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for ending in ["local-close", "remote-close", "eof", "disconnect"] {
            // A one-byte window never replenishes: russh compares remaining < target / 2.
            let mut f = Fixture::with_window(1).await;
            let (mut input_task, input) = super::super::input::start(Arc::clone(&f._writer));
            let (tx, mut write_reply) = oneshot::channel();
            input.send((vec![b'x'; 8192], tx)).await.unwrap();
            f.expect_data(b"x").await;
            assert!(matches!(
                write_reply.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            f.server
                .data(f.id, CryptoVec::from_slice(b"output while input stalled"))
                .await
                .unwrap();
            f.expect_data(b"output while input stalled").await;
            // Fill the separate bounded input queue. Close must remain available.
            let mut queued = Vec::new();
            for _ in 0..32 {
                let (tx, rx) = oneshot::channel();
                input.try_send((vec![b'q'], tx)).unwrap();
                queued.push(rx);
            }
            let (tx, _rx) = oneshot::channel();
            assert!(matches!(
                input.try_send((vec![b'z'], tx)),
                Err(mpsc::error::TrySendError::Full(_))
            ));
            let (tx, _rx) = oneshot::channel();
            let mut blocked_send = Box::pin(input.send((vec![b'z'], tx)));
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut blocked_send)
                    .await
                    .is_err()
            );
            match ending {
                "eof" => {
                    f.server.eof(f.id).await.unwrap();
                    f.expect_end().await;
                }
                "disconnect" => {
                    f.server
                        .disconnect(russh::Disconnect::ByApplication, "done".into(), "en".into())
                        .await
                        .unwrap();
                    f.expect_end().await;
                }
                _ => {
                    if ending == "remote-close" {
                        f.server.close(f.id).await.unwrap();
                        assert!(matches!(f.next().await, Some(ShellEvent::ChannelClosed)));
                        input_task.stop().await;
                        // The peer must receive our CLOSE acknowledgement even with pending input.
                        assert_eq!(f.closed.recv().await, Some(f.id));
                    }
                    let (tx, rx) = oneshot::channel();
                    f.sender.send(SshControl::Close(tx)).await.unwrap();
                    let Some(ShellEvent::Control(control)) = f.next().await else {
                        panic!("close lost")
                    };
                    input_task.stop().await;
                    let routes = Arc::new(std::sync::Mutex::new(Default::default()));
                    assert!(
                        !super::super::handle_control(
                            &mut f.handle,
                            &f._writer,
                            &mut None,
                            "fixture",
                            &routes,
                            control
                        )
                        .await
                    );
                    assert!(rx.await.unwrap().is_ok());
                }
            }
            drop(input_task);
            assert!(
                write_reply.await.is_err(),
                "blocked input waiter must be released"
            );
            for reply in queued {
                assert!(reply.await.is_err());
            }
            assert!(
                blocked_send.await.is_err(),
                "full-queue sender must be released"
            );
            assert!(input.is_closed());
            f.finish().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_writes_preserve_bytes_and_order_across_window_adjustments() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut f = Fixture::with_window(2048).await;
        let (input_task, input) = super::super::input::start(Arc::clone(&f._writer));
        let first: Vec<u8> = (0..16384).map(|i| (i % 256) as u8).collect();
        let second = b"second input after binary data".to_vec();
        for data in [&first, &second] {
            let (tx, rx) = oneshot::channel();
            input.send((data.clone(), tx)).await.unwrap();
            assert!(rx.await.unwrap().is_ok());
        }
        let expected = [first, second].concat();
        let mut output = Vec::new();
        while output.len() < expected.len() {
            match f.next().await {
                Some(ShellEvent::Channel(ChannelMsg::Data { data })) => {
                    output.extend_from_slice(&data)
                }
                Some(ShellEvent::Channel(ChannelMsg::WindowAdjusted { .. })) => {}
                _ => panic!("expected echoed bytes"),
            }
        }
        assert_eq!(output, expected);
        drop(input_task);
        f.finish().await;
    })
    .await
    .unwrap();
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
        assert!(matches!(f.next().await, Some(ShellEvent::ChannelClosed)));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), f.next())
                .await
                .is_err(),
            "CLOSE alone must not end the shell"
        );
        assert!(f.channel_closed);
        let another = f.handle.channel_open_session().await.unwrap();
        another.close().await.unwrap();
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
            resize_rx.try_recv(),
            Ok(Err(super::super::SshError::Closed))
        ));
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_close_precedes_ready_controls() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut f = Fixture::new().await;
        for _ in 0..32 {
            f.channel_closed = false;
            f.pending.push_back(ChannelMsg::Close);
            let (close_tx, _close_rx) = oneshot::channel();
            f.sender.send(SshControl::Close(close_tx)).await.unwrap();
            assert!(matches!(f.next().await, Some(ShellEvent::ChannelClosed)));
            assert!(matches!(
                f.next().await,
                Some(ShellEvent::Control(SshControl::Close(_)))
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
        f.server
            .exit_signal_request(f.id, russh::Sig::TERM, false, "fixture".into(), "en".into())
            .await
            .unwrap();
        f.server
            .data(f.id, CryptoVec::from_slice(b"after status and signal"))
            .await
            .unwrap();
        f.expect_data(b"after status and signal").await;
        let (tx, _rx) = oneshot::channel();
        f.sender.send(SshControl::Close(tx)).await.unwrap();
        assert!(matches!(
            f.next().await,
            Some(ShellEvent::Control(SshControl::Close(_)))
        ));
        f.finish().await;
    })
    .await
    .unwrap();
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
            f.expect_end().await;
            f.finish().await;
        }
    })
    .await
    .unwrap();
}

#[path = "operation_tests.rs"]
mod operation_tests;

#[path = "native_completion_tests.rs"]
mod native_completion_tests;

#[path = "terminal_race_tests.rs"]
mod terminal_race_tests;

#[path = "resize_race_tests.rs"]
mod resize_race_tests;
