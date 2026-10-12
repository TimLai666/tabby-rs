use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use russh::{client, server, Channel, ChannelId, CryptoVec};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};

use super::{
    forwarding,
    model::{SshForwardingInfo, SshForwardingStatus, SshForwardingType},
    ForwardingRuntime, SshControl, SshManager,
};

const LIMIT: Duration = Duration::from_secs(5);

fn info(id: &str, session_id: &str, kind: SshForwardingType) -> SshForwardingInfo {
    SshForwardingInfo {
        id: id.into(),
        session_id: session_id.into(),
        kind,
        bind_host: "127.0.0.1".into(),
        bind_port: 0,
        target_address: "127.0.0.1".into(),
        target_port: 9,
        status: SshForwardingStatus::Active,
        last_error: None,
    }
}

fn manager() -> SshManager {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    SshManager::new(std::env::temp_dir().join(format!(
        "tabby-forwarding-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}

fn add_runtime(manager: &SshManager, item: SshForwardingInfo) -> super::pending::CancelSignal {
    let runtime = ForwardingRuntime::new(item.clone());
    let cancel = runtime.cancel.clone();
    manager.forwardings.lock().unwrap().insert(item.id, runtime);
    cancel
}

struct AbortTask<T>(Option<JoinHandle<T>>);
impl<T> Drop for AbortTask<T> {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}
fn spawn<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> AbortTask<T> {
    AbortTask(Some(tokio::spawn(future)))
}
impl<T> AbortTask<T> {
    async fn bounded(mut self) -> T {
        let task = self.0.as_mut().unwrap();
        match timeout(LIMIT, task).await {
            Ok(Ok(output)) => {
                self.0.take();
                output
            }
            Ok(Err(error)) => {
                self.0.take();
                panic!("task failed: {error}")
            }
            Err(_) => {
                if let Some(task) = self.0.take() {
                    task.abort();
                    let _ = task.await;
                }
                panic!("task exceeded {LIMIT:?}")
            }
        }
    }
}
async fn finish(task: AbortTask<()>) {
    task.bounded().await;
}
async fn connect_tcp(address: std::net::SocketAddr) -> TcpStream {
    timeout(LIMIT, TcpStream::connect(address))
        .await
        .unwrap()
        .unwrap()
}

struct Pinned(russh::keys::PublicKey);
impl client::Handler for Pinned {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.0.key_data())
    }
}

struct EchoServer {
    opened: mpsc::UnboundedSender<ChannelId>,
    closed: mpsc::UnboundedSender<ChannelId>,
    gate: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
}
impl server::Handler for EchoServer {
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
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        _: &str,
        _: u32,
        _: &str,
        _: u32,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let id = channel.id();
        let _ = self.opened.send(id);
        let gate = { self.gate.lock().unwrap().take() };
        if let Some(gate) = gate {
            let _ = timeout(LIMIT, gate).await;
        }
        Ok(true)
    }
    async fn channel_close(
        &mut self,
        id: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = self.closed.send(id);
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        bytes: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.data(id, CryptoVec::from_slice(bytes))
    }
}

struct Fixture {
    handle: client::Handle<Pinned>,
    server_task: JoinHandle<()>,
    opened: mpsc::UnboundedReceiver<ChannelId>,
    closed: mpsc::UnboundedReceiver<ChannelId>,
    writer: russh::ChannelWriteHalf<client::Msg>,
    _reader: russh::ChannelReadHalf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server_task.abort();
    }
}
impl Fixture {
    async fn new(gate: Option<oneshot::Receiver<()>>) -> Self {
        let key = russh::keys::PrivateKey::random(
            &mut rand::rngs::OsRng,
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public = key.public_key().clone();
        let mut config = server::Config::default();
        config.keys.push(key);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (opened_tx, opened) = mpsc::unbounded_channel();
        let (closed_tx, closed) = mpsc::unbounded_channel();
        let server_task = tokio::spawn(async move {
            let (tcp, _) = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
            let handler = EchoServer {
                opened: opened_tx,
                closed: closed_tx,
                gate: Arc::new(Mutex::new(gate)),
            };
            if let Ok(session) = server::run_stream(Arc::new(config), tcp, handler).await {
                let _ = session.await;
            }
        });
        let mut handle = timeout(
            LIMIT,
            client::connect(Arc::new(client::Config::default()), address, Pinned(public)),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(timeout(LIMIT, handle.authenticate_none("fixture"))
            .await
            .unwrap()
            .unwrap()
            .success());
        let session = timeout(LIMIT, handle.channel_open_session())
            .await
            .unwrap()
            .unwrap();
        let (reader, writer) = session.split();
        Self {
            handle,
            server_task,
            opened,
            closed,
            writer,
            _reader: reader,
        }
    }
    async fn direct(&mut self) -> Channel<client::Msg> {
        // This exercises a real SSH direct-tcpip channel as the helper's wire peer. It does not
        // issue the remote-forward global request used by an actual SSH server route.
        timeout(
            LIMIT,
            self.handle
                .channel_open_direct_tcpip("127.0.0.1", 9, "127.0.0.1", 0),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn cleanup(mut self) {
        let _ = timeout(
            LIMIT,
            self.handle
                .disconnect(russh::Disconnect::ByApplication, "fixture complete", "en"),
        )
        .await;
        match timeout(LIMIT, &mut self.server_task).await {
            Ok(_) => {}
            Err(_) => {
                self.server_task.abort();
                let _ = (&mut self.server_task).await;
            }
        }
    }
}

async fn read_channel_echo(channel: Channel<client::Msg>, payload: &[u8]) {
    let mut stream = channel.into_stream();
    stream.write_all(payload).await.unwrap();
    let mut reply = vec![0; payload.len()];
    timeout(LIMIT, stream.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, payload);
}

#[tokio::test]
async fn local_and_dynamic_cancel_before_first_poll_release_exact_port() {
    for (id, kind) in [
        ("local", SshForwardingType::Local),
        ("dynamic", SshForwardingType::Dynamic),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let manager = manager();
        let item = info(id, id, kind);
        let cancel = add_runtime(&manager, item.clone());
        manager.stop_forwardings_for_session(id, "conn");
        let (tx, _rx) = mpsc::channel(2);
        let task = spawn(forwarding::run_local_forward(item, listener, tx, cancel));
        finish(task).await;
        assert_eq!(
            timeout(LIMIT, TcpListener::bind(address))
                .await
                .unwrap()
                .unwrap()
                .local_addr()
                .unwrap(),
            address
        );
    }
}

#[tokio::test]
async fn socks_greeting_then_pending_request_is_closed_on_cancel() {
    let manager = manager();
    let item = info("dyn", "owner", SshForwardingType::Dynamic);
    let cancel = add_runtime(&manager, item.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, mut rx) = mpsc::channel(2);
    let task = spawn(forwarding::run_local_forward(item, listener, tx, cancel));
    let mut peer = connect_tcp(address).await;
    peer.write_all(&[5, 1]).await.unwrap();
    // The server is waiting for the method byte; completing the greeting synchronizes its response.
    peer.write_all(&[0]).await.unwrap();
    let mut reply = [0; 2];
    timeout(LIMIT, peer.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, [5, 0]);
    manager.stop_forwardings_for_session("owner", "conn");
    let mut byte = [0];
    assert_eq!(
        timeout(LIMIT, peer.read(&mut byte)).await.unwrap().unwrap(),
        0
    );
    assert!(rx.try_recv().is_err());
    finish(task).await;
}

#[tokio::test]
async fn incomplete_socks_greeting_is_closed_on_cancel() {
    let manager = manager();
    let item = info(
        "dyn-incomplete",
        "owner-incomplete",
        SshForwardingType::Dynamic,
    );
    let cancel = add_runtime(&manager, item.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, _rx) = mpsc::channel(2);
    let task = spawn(forwarding::run_local_forward(item, listener, tx, cancel));
    let mut peer = connect_tcp(address).await;
    peer.write_all(&[5, 1]).await.unwrap();
    manager.stop_forwardings_for_session("owner-incomplete", "conn");
    let mut byte = [0];
    // Closing with unread greeting bytes can reset TCP instead of delivering a graceful EOF.
    match timeout(LIMIT, peer.read(&mut byte)).await.unwrap() {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("cancelled SOCKS peer stayed open or failed unexpectedly: {other:?}"),
    }
    finish(task).await;
}

#[tokio::test]
async fn pending_local_open_reply_cancellation_closes_peer_and_drops_receiver() {
    let manager = manager();
    let item = info("local", "owner", SshForwardingType::Local);
    let cancel = add_runtime(&manager, item.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, mut rx) = mpsc::channel(2);
    let task = spawn(forwarding::run_local_forward(item, listener, tx, cancel));
    let mut peer = connect_tcp(address).await;
    let command = timeout(LIMIT, rx.recv()).await.unwrap().unwrap();
    let sender = match command {
        SshControl::OpenDirectTcpip { sender, .. } => sender,
        _ => panic!("expected direct open"),
    };
    manager.stop_forwardings_for_session("owner", "conn");
    let mut byte = [0];
    assert_eq!(
        timeout(LIMIT, peer.read(&mut byte)).await.unwrap().unwrap(),
        0
    );
    finish(task).await;
    assert!(sender.is_closed());
}

#[tokio::test]
async fn removing_runtime_cancels_pending_child_peer() {
    let manager = manager();
    let item = info("local", "owner", SshForwardingType::Local);
    let cancel = add_runtime(&manager, item.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, mut rx) = mpsc::channel(2);
    let task = spawn(forwarding::run_local_forward(item, listener, tx, cancel));
    let mut peer = connect_tcp(address).await;
    let _open = timeout(LIMIT, rx.recv()).await.unwrap().unwrap();
    manager.forwardings.lock().unwrap().remove("local");
    let mut byte = [0];
    assert_eq!(
        timeout(LIMIT, peer.read(&mut byte)).await.unwrap().unwrap(),
        0
    );
    finish(task).await;
}

#[tokio::test]
async fn session_stop_isolated_and_other_local_binary_relay_survives() {
    let manager = manager();
    let a = info("a", "owner-a", SshForwardingType::Local);
    let b = info("b", "owner-b", SshForwardingType::Local);
    let ca = add_runtime(&manager, a.clone());
    let cb = add_runtime(&manager, b.clone());
    let la = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let aa = la.local_addr().unwrap();
    let lb = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let ab = lb.local_addr().unwrap();
    let (ta, mut ra) = mpsc::channel(2);
    let (tb, mut rb) = mpsc::channel(2);
    let ja = spawn(forwarding::run_local_forward(a, la, ta, ca));
    let jb = spawn(forwarding::run_local_forward(b, lb, tb, cb));
    let mut pa = connect_tcp(aa).await;
    let _ = timeout(LIMIT, ra.recv()).await.unwrap().unwrap();
    manager.stop_forwardings_for_session("owner-a", "conn-a");
    finish(ja).await;
    let mut eof = [0];
    assert_eq!(timeout(LIMIT, pa.read(&mut eof)).await.unwrap().unwrap(), 0);
    let mut pb = connect_tcp(ab).await;
    let command = timeout(LIMIT, rb.recv()).await.unwrap().unwrap();
    let sender = match command {
        SshControl::OpenDirectTcpip { sender, .. } => sender,
        _ => panic!("expected direct open"),
    };
    let mut fixture = Fixture::new(None).await;
    let channel = fixture.direct().await;
    let _opened = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(sender.send(Ok(channel.into_stream())).is_ok());
    let bytes = [0xde, 0x00, 0xad, 0xff, 0xbe];
    pb.write_all(&bytes).await.unwrap();
    let mut echoed = [0; 5];
    timeout(LIMIT, pb.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, bytes);
    manager.stop_forwardings_for_session("owner-b", "conn-b");
    finish(jb).await;
    fixture.cleanup().await;
}

#[tokio::test]
async fn queued_late_direct_open_after_dropped_receiver_is_not_started() {
    let mut fixture = Fixture::new(None).await;
    let survivor = fixture.direct().await;
    let _survivor_id = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    let payload = [0x91, 0, 0xfe, 0x03];
    read_channel_echo(survivor, &payload).await;
    let (tx, rx) = oneshot::channel();
    drop(rx);
    let keep = super::handle_control(
        &mut fixture.handle,
        &fixture.writer,
        &mut None,
        "conn",
        &Arc::new(Mutex::new(Default::default())),
        SshControl::OpenDirectTcpip {
            host: "127.0.0.1".into(),
            port: 9,
            cancel: super::pending::CancelSignal::default(),
            sender: tx,
        },
    )
    .await;
    assert!(keep);
    assert!(timeout(Duration::from_millis(250), fixture.opened.recv())
        .await
        .is_err());
    fixture.cleanup().await;
}

#[tokio::test]
async fn late_direct_open_reply_after_receiver_drop_closes_ssh_channel() {
    let (release, gate) = oneshot::channel();
    let mut fixture = Fixture::new(Some(gate)).await;
    let (tx, rx) = oneshot::channel();
    let routes = Arc::new(Mutex::new(Default::default()));
    let mut sftp = None;
    let control = super::handle_control(
        &mut fixture.handle,
        &fixture.writer,
        &mut sftp,
        "conn",
        &routes,
        SshControl::OpenDirectTcpip {
            host: "127.0.0.1".into(),
            port: 9,
            cancel: super::pending::CancelSignal::default(),
            sender: tx,
        },
    );
    let cancellation = async {
        let id = timeout(LIMIT, fixture.opened.recv())
            .await
            .unwrap()
            .unwrap();
        drop(rx);
        release.send(()).unwrap();
        id
    };
    let (keep, id) = timeout(LIMIT, async { tokio::join!(control, cancellation) })
        .await
        .unwrap();
    assert!(keep);
    assert_eq!(
        timeout(LIMIT, fixture.closed.recv())
            .await
            .unwrap()
            .unwrap(),
        id
    );
    let channel = fixture.direct().await;
    let payload = [1, 2, 0, 255];
    read_channel_echo(channel, &payload).await;
    fixture.cleanup().await;
}

#[tokio::test]
async fn cancelled_remote_route_does_not_connect_target_and_closes_channel() {
    let manager = manager();
    let item = info("remote", "remote-owner", SshForwardingType::Remote);
    let cancel = add_runtime(&manager, item);
    manager.stop_forwardings_for_session("remote-owner", "conn");
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = target.local_addr().unwrap();
    let mut fixture = Fixture::new(None).await;
    let channel = fixture.direct().await;
    let id = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    let route = super::RemoteForwardRoute {
        target_address: address.ip().to_string(),
        target_port: address.port(),
        cancel,
        ready: None,
    };
    timeout(
        LIMIT,
        forwarding::run_remote_forward(
            channel.into_stream(),
            Some(Arc::new(route)),
            "127.0.0.1",
            0,
            |_| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(timeout(Duration::from_millis(250), target.accept())
        .await
        .is_err());
    assert_eq!(
        timeout(LIMIT, fixture.closed.recv())
            .await
            .unwrap()
            .unwrap(),
        id
    );
    fixture.cleanup().await;
}

#[tokio::test]
async fn active_remote_route_cancel_closes_target_and_other_route_keeps_binary_relay() {
    let mut fixture = Fixture::new(None).await;
    let target_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr_a = target_a.local_addr().unwrap();
    let target_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr_b = target_b.local_addr().unwrap();
    let manager = manager();
    let ca = add_runtime(&manager, info("ra", "owner-a", SshForwardingType::Remote));
    let cb = add_runtime(&manager, info("rb", "owner-b", SshForwardingType::Remote));
    let channel_a = fixture.direct().await;
    let id_a = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    let channel_b = fixture.direct().await;
    let id_b = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    let route_a = super::RemoteForwardRoute {
        target_address: addr_a.ip().to_string(),
        target_port: addr_a.port(),
        cancel: ca.clone(),
        ready: None,
    };
    let route_b = super::RemoteForwardRoute {
        target_address: addr_b.ip().to_string(),
        target_port: addr_b.port(),
        cancel: cb.clone(),
        ready: None,
    };
    let ja = spawn(forwarding::run_remote_forward(
        channel_a.into_stream(),
        Some(Arc::new(route_a)),
        "127.0.0.1",
        0,
        |_| {},
    ));
    let jb = spawn(forwarding::run_remote_forward(
        channel_b.into_stream(),
        Some(Arc::new(route_b)),
        "127.0.0.1",
        0,
        |_| {},
    ));
    let (mut peer_a, _) = timeout(LIMIT, target_a.accept()).await.unwrap().unwrap();
    let (mut peer_b, _) = timeout(LIMIT, target_b.accept()).await.unwrap().unwrap();
    manager.stop_forwardings_for_session("owner-a", "conn-a");
    let mut eof = [0];
    assert_eq!(
        timeout(LIMIT, peer_a.read(&mut eof))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(
        timeout(LIMIT, fixture.closed.recv())
            .await
            .unwrap()
            .unwrap(),
        id_a
    );
    assert!(ja.bounded().await.is_ok());
    let payload = [0xfa, 0, 1, 0xfe];
    peer_b.write_all(&payload).await.unwrap();
    let mut received = [0; 4];
    timeout(LIMIT, peer_b.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, payload);
    // The other direct channel remains independently usable through its target relay.
    peer_b.shutdown().await.unwrap();
    manager.stop_forwardings_for_session("owner-b", "conn-b");
    assert!(jb.bounded().await.is_ok());
    let _ = id_b;
    fixture.cleanup().await;
}

#[tokio::test]
async fn delivered_direct_open_is_closed_if_receiver_is_dropped_before_poll() {
    let mut fixture = Fixture::new(None).await;
    let (sender, receiver) = oneshot::channel();
    assert!(
        super::handle_control(
            &mut fixture.handle,
            &fixture.writer,
            &mut None,
            "conn",
            &Arc::new(Mutex::new(Default::default())),
            SshControl::OpenDirectTcpip {
                host: "127.0.0.1".into(),
                port: 9,
                cancel: super::pending::CancelSignal::default(),
                sender,
            },
        )
        .await
    );
    let id = timeout(LIMIT, fixture.opened.recv())
        .await
        .unwrap()
        .unwrap();
    // Delivery succeeded, but cancellation can drop the unpolled receiver and its payload.
    drop(receiver);
    assert_eq!(
        timeout(LIMIT, fixture.closed.recv())
            .await
            .unwrap()
            .unwrap(),
        id
    );
    let survivor = fixture.direct().await;
    read_channel_echo(survivor, &[0x12, 0, 0xff, 0x34]).await;
    fixture.cleanup().await;
}

#[tokio::test]
async fn cancelled_queued_direct_open_is_not_started_with_live_receiver() {
    let mut fixture = Fixture::new(None).await;
    let cancel = super::pending::CancelSignal::default();
    let (sender, mut receiver) = oneshot::channel();
    // The child has not polled cancellation, so the reply receiver is still alive.
    cancel.cancel();
    assert!(
        super::handle_control(
            &mut fixture.handle,
            &fixture.writer,
            &mut None,
            "conn",
            &Arc::new(Mutex::new(Default::default())),
            SshControl::OpenDirectTcpip {
                host: "127.0.0.1".into(),
                port: 9,
                cancel,
                sender,
            },
        )
        .await
    );
    assert!(timeout(Duration::from_millis(250), fixture.opened.recv())
        .await
        .is_err());
    assert!(matches!(
        receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));
    fixture.cleanup().await;
}
