use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Wake, Waker},
    time::Duration,
};

use russh::{client, server, Channel, ChannelId, ChannelMsg, CryptoVec};
use tokio::{
    io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, Notify},
    task::JoinHandle,
    time::timeout,
};

use super::{
    forwarding,
    model::{SshForwardingInfo, SshForwardingStatus, SshForwardingType},
    pending::CancelSignal,
    ForwardingRuntime, RemoteForwardRoute, SshControl, SshManager,
};

const LIMIT: Duration = Duration::from_secs(5);
type Routes = super::RemoteForwardRoutes;

fn key_pair() -> (russh::keys::PrivateKey, russh::keys::PublicKey) {
    let private =
        russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
            .unwrap();
    let public = private.public_key().clone();
    (private, public)
}

struct PinnedClient {
    key: russh::keys::PublicKey,
    routes: Routes,
    connection_id: String,
}

impl client::Handler for PinnedClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(key.key_data() == self.key.key_data())
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        _: &str,
        _: u32,
        _: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        forwarding::dispatch_remote_forward(
            channel,
            &self.routes,
            &self.connection_id,
            connected_address,
            connected_port,
        );
        async { Ok(()) }
    }
}

struct ListenerEntry {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct ServerState {
    listeners: Mutex<HashMap<(String, u32), ListenerEntry>>,
    relays: Arc<Mutex<Vec<JoinHandle<()>>>>,
    accepted_requests: Mutex<Option<mpsc::UnboundedSender<(String, u32)>>>,
    rejected_requests: Mutex<Option<mpsc::UnboundedSender<(String, u32)>>>,
    close_events: Mutex<Option<mpsc::UnboundedSender<ChannelId>>>,
    closed_channels: Mutex<Vec<ChannelId>>,
    auto_eager: AtomicBool,
    reject_next: AtomicBool,
    auto_results: Mutex<Vec<(ChannelId, bool)>>,
    auto_changed: Notify,
}

impl ServerState {
    fn stop_all(&self) {
        for (_, entry) in self
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain()
        {
            let _ = entry.stop.send(());
            entry.task.abort();
        }
        for task in self
            .relays
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain(..)
        {
            task.abort();
        }
    }
}

struct WireServer {
    state: Arc<ServerState>,
    shell_handle: Option<oneshot::Sender<server::Handle>>,
    shell_channel: Option<ChannelId>,
}

impl server::Handler for WireServer {
    type Error = russh::Error;

    async fn auth_none(&mut self, _: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        self.shell_channel = Some(channel.id());
        if let Some(sender) = self.shell_handle.take() {
            let _ = sender.send(session.handle());
        }
        Ok(true)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if self.shell_channel == Some(channel) {
            session.data(channel, CryptoVec::from_slice(data))?;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.state
            .closed_channels
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(channel);
        if let Some(sender) = self
            .state
            .close_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            let _ = sender.send(channel);
        }
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        if self.state.reject_next.swap(false, Ordering::Relaxed) {
            if let Some(sender) = self
                .state
                .rejected_requests
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
            {
                let _ = sender.send((address.to_owned(), *port));
            }
            return Ok(false);
        }
        let requested_port = u16::try_from(*port).map_err(|_| russh::Error::Inconsistent)?;
        let listener = match TcpListener::bind((address, requested_port)).await {
            Ok(listener) => listener,
            Err(_) => return Ok(false),
        };
        let actual = listener.local_addr().map_err(russh::Error::from)?;
        *port = u32::from(actual.port());
        let key = (address.to_owned(), *port);
        let (stop_tx, mut stop_rx) = oneshot::channel();
        let server_handle = session.handle();
        let task_relays = self.state.relays.clone();
        let wire_address = address.to_owned();
        let wire_port = *port;
        let accept_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut socket, origin)) = accepted else { break; };
                        let handle = server_handle.clone();
                        let address = wire_address.clone();
                        let relay = tokio::spawn(async move {
                            if let Ok(channel) = handle.channel_open_forwarded_tcpip(address, wire_port, origin.ip().to_string(), u32::from(origin.port())).await {
                                let mut stream = channel.into_stream();
                                let _ = copy_bidirectional(&mut socket, &mut stream).await;
                            }
                        });
                        task_relays.lock().unwrap_or_else(|error| error.into_inner()).push(relay);
                    }
                }
            }
        });
        self.state
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(
                key,
                ListenerEntry {
                    stop: stop_tx,
                    task: accept_task,
                },
            );
        if let Some(sender) = self
            .state
            .accepted_requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            let _ = sender.send((address.to_owned(), *port));
        }
        if self.state.auto_eager.load(Ordering::Relaxed) {
            let handle = session.handle();
            let address = address.to_owned();
            let port = *port;
            let state = self.state.clone();
            let task = tokio::spawn(async move {
                let Ok(channel) = handle
                    .channel_open_forwarded_tcpip(address, port, "127.0.0.1", 1234)
                    .await
                else {
                    return;
                };
                let id = channel.id();
                let mut stream = channel.into_stream();
                let bytes = [0x51, 0, 0xe1, 0xff];
                let echoed = async {
                    stream.write_all(&bytes).await?;
                    let mut echoed = [0; 4];
                    stream.read_exact(&mut echoed).await?;
                    Ok::<bool, std::io::Error>(echoed == bytes)
                }
                .await
                .unwrap_or(false);
                let accepted = echoed;
                state
                    .auto_results
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push((id, accepted));
                state.auto_changed.notify_waiters();
            });
            self.state
                .relays
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(task);
        }
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let entry = self
            .state
            .listeners
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&(address.to_owned(), port));
        let Some(entry) = entry else { return Ok(false) };
        let _ = entry.stop.send(());
        match timeout(LIMIT, entry.task).await {
            Ok(Ok(())) => Ok(true),
            _ => Ok(false),
        }
    }
}

struct Peer {
    connection_id: String,
    routes: Routes,
    handle: client::Handle<PinnedClient>,
    server_handle: server::Handle,
    writer: russh::ChannelWriteHalf<client::Msg>,
    reader: russh::ChannelReadHalf,
    state: Arc<ServerState>,
    accepted_requests: mpsc::UnboundedReceiver<(String, u32)>,
    rejected_requests: mpsc::UnboundedReceiver<(String, u32)>,
    close_events: mpsc::UnboundedReceiver<ChannelId>,
    server_task: Option<JoinHandle<()>>,
}

impl Peer {
    async fn connect(routes: Routes, connection_id: &str) -> Self {
        let (private, public) = key_pair();
        let mut config = server::Config::default();
        config.keys.push(private);
        let ssh_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let ssh_address = ssh_listener.local_addr().unwrap();
        let state = Arc::new(ServerState::default());
        let (accepted_sender, accepted_requests) = mpsc::unbounded_channel();
        let (rejected_sender, rejected_requests) = mpsc::unbounded_channel();
        let (close_sender, close_events) = mpsc::unbounded_channel();
        *state
            .accepted_requests
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(accepted_sender);
        *state
            .rejected_requests
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(rejected_sender);
        *state
            .close_events
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(close_sender);
        let server_state = state.clone();
        let (server_handle_tx, server_handle_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            let accepted = timeout(LIMIT, ssh_listener.accept()).await;
            let Ok(Ok((tcp, _))) = accepted else { return };
            let handler = WireServer {
                state: server_state,
                shell_handle: Some(server_handle_tx),
                shell_channel: None,
            };
            if let Ok(session) = server::run_stream(Arc::new(config), tcp, handler).await {
                let _ = session.await;
            }
        });
        let mut handle = timeout(
            LIMIT,
            client::connect(
                Arc::new(client::Config::default()),
                ssh_address,
                PinnedClient {
                    key: public,
                    routes: routes.clone(),
                    connection_id: connection_id.into(),
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(handle.authenticate_none("fixture").await.unwrap().success());
        let shell = timeout(LIMIT, handle.channel_open_session())
            .await
            .unwrap()
            .unwrap();
        let server_handle = timeout(LIMIT, server_handle_rx).await.unwrap().unwrap();
        let (reader, writer) = shell.split();
        Self {
            connection_id: connection_id.into(),
            routes,
            handle,
            server_handle,
            writer,
            reader,
            state,
            accepted_requests,
            rejected_requests,
            close_events,
            server_task: Some(server_task),
        }
    }

    async fn start_remote(
        &mut self,
        bind_port: u16,
        target: SocketAddr,
        cancel: CancelSignal,
    ) -> u16 {
        let (sender, receiver) = oneshot::channel();
        let keep = super::handle_control(
            &mut self.handle,
            &self.writer,
            &mut None,
            &self.connection_id,
            &self.routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port,
                target_address: target.ip().to_string(),
                target_port: target.port(),
                cancel,
                sender,
            },
        )
        .await;
        assert!(keep, "remote start must keep the SSH session alive");
        receiver.await.unwrap().unwrap()
    }

    async fn stop_remote(&mut self, address: &str, port: u16) {
        let (sender, receiver) = oneshot::channel();
        let keep = super::handle_control(
            &mut self.handle,
            &self.writer,
            &mut None,
            &self.connection_id,
            &self.routes,
            SshControl::StopRemoteForward {
                bind_host: address.into(),
                bind_port: port,
                sender,
            },
        )
        .await;
        assert!(keep, "remote stop must keep the SSH session alive");
        receiver.await.unwrap().unwrap();
    }

    async fn shell_echo(&mut self, bytes: &[u8]) {
        let mut writer = self.writer.make_writer();
        writer.write_all(bytes).await.unwrap();
        writer.flush().await.unwrap();
        let mut received = Vec::new();
        timeout(LIMIT, async {
            while received.len() < bytes.len() {
                match self.reader.wait().await {
                    Some(ChannelMsg::Data { data }) => received.extend_from_slice(&data),
                    Some(message) => panic!("unexpected shell message: {message:?}"),
                    None => panic!("shell channel closed before echo"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(received, bytes);
    }

    async fn open_forwarded(&self, address: &str, port: u32) -> Channel<server::Msg> {
        timeout(
            LIMIT,
            self.server_handle
                .channel_open_forwarded_tcpip(address, port, "127.0.0.1", 1234),
        )
        .await
        .unwrap()
        .unwrap()
    }

    async fn cleanup(mut self) {
        self.state.stop_all();
        let _ = timeout(
            LIMIT,
            self.handle
                .disconnect(russh::Disconnect::ByApplication, "fixture done", "en"),
        )
        .await;
        if let Some(mut task) = self.server_task.take() {
            if timeout(LIMIT, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.state.stop_all();
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
    }
}

async fn reserve_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn echo_remote(remote_port: u16, target: &TcpListener, bytes: &[u8]) {
    let mut remote = timeout(LIMIT, TcpStream::connect(("127.0.0.1", remote_port)))
        .await
        .unwrap()
        .unwrap();
    let (mut target_socket, _) = timeout(LIMIT, target.accept()).await.unwrap().unwrap();
    remote.write_all(bytes).await.unwrap();
    let mut at_target = vec![0; bytes.len()];
    timeout(LIMIT, target_socket.read_exact(&mut at_target))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(at_target, bytes);
    target_socket.write_all(bytes).await.unwrap();
    let mut echoed = vec![0; bytes.len()];
    timeout(LIMIT, remote.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, bytes);
}

async fn echo_on_channel(channel: Channel<server::Msg>, target: &TcpListener, bytes: &[u8]) {
    let mut remote = channel.into_stream();
    let (mut target_socket, _) = timeout(LIMIT, target.accept()).await.unwrap().unwrap();
    remote.write_all(bytes).await.unwrap();
    let mut received = vec![0; bytes.len()];
    timeout(LIMIT, target_socket.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received, bytes);
    target_socket.write_all(bytes).await.unwrap();
    let mut echoed = vec![0; bytes.len()];
    timeout(LIMIT, remote.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, bytes);
}

async fn assert_server_channel_closed(channel: Channel<server::Msg>) {
    let mut stream = channel.into_stream();
    let mut byte = [0];
    let result = timeout(LIMIT, stream.read(&mut byte)).await.unwrap();
    assert!(
        result.map(|count| count == 0).unwrap_or(true),
        "unknown route channel must close"
    );
}

struct ReplyWake {
    ready: AtomicBool,
    notify: Notify,
}

impl Wake for ReplyWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.ready.store(true, Ordering::Release);
        self.notify.notify_one();
    }
}

fn poll_control_once<F: Future>(future: std::pin::Pin<&mut F>, wake: Arc<ReplyWake>) {
    let waker = Waker::from(wake);
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(tokio::task::unconstrained(future));
    assert!(future.as_mut().poll(&mut context).is_pending());
}

async fn wait_for_wake(wake: &ReplyWake) {
    timeout(LIMIT, async {
        while !wake.ready.load(Ordering::Acquire) {
            let notified = wake.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if wake.ready.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    })
    .await
    .expect("control request should wake its first poll");
}

async fn wait_for_accepted_request(
    requests: &mut mpsc::UnboundedReceiver<(String, u32)>,
    requested_host: &str,
    requested_port: u16,
) -> u32 {
    timeout(LIMIT, async {
        loop {
            let (host, port) = requests.recv().await.expect("peer request observer closed");
            if host == requested_host && (requested_port == 0 || u32::from(requested_port) == port)
            {
                return port;
            }
        }
    })
    .await
    .expect("peer must accept this exact forwarded-port request")
}

async fn wait_for_channel_close(
    close_events: &mut mpsc::UnboundedReceiver<ChannelId>,
    state: &ServerState,
    channel_id: ChannelId,
) {
    if state
        .closed_channels
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(&channel_id)
    {
        return;
    }
    timeout(LIMIT, async {
        loop {
            match close_events
                .recv()
                .await
                .expect("peer closed the CHANNEL_CLOSE observer before the expected channel")
            {
                received if received == channel_id => return,
                _ => {}
            }
        }
    })
    .await
    .expect("peer must decode CHANNEL_CLOSE for the expected channel");
    assert!(state
        .closed_channels
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(&channel_id));
}

async fn assert_channel_waits_without_target(
    close_events: &mut mpsc::UnboundedReceiver<ChannelId>,
    channel: &mut Channel<server::Msg>,
    target: &TcpListener,
) {
    let id = channel.id();
    let (closed, accepted) = tokio::join!(
        timeout(Duration::from_millis(250), async {
            loop {
                if close_events
                    .recv()
                    .await
                    .expect("peer closed the CHANNEL_CLOSE observer unexpectedly")
                    == id
                {
                    return;
                }
            }
        }),
        timeout(Duration::from_millis(250), target.accept()),
    );
    assert!(
        closed.is_err(),
        "early channel closed before its request resolved"
    );
    assert!(
        accepted.is_err(),
        "pending route connected to its target early"
    );
}

async fn wait_for_auto_result(peer: &Peer, index: usize) -> (ChannelId, bool) {
    timeout(LIMIT, async {
        loop {
            let changed = peer.state.auto_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(result) = peer
                .state
                .auto_results
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(index)
                .copied()
            {
                return result;
            }
            changed.await;
        }
    })
    .await
    .expect("automatic eager channel should finish its binary echo")
}

#[tokio::test]
async fn specified_and_allocated_remote_ports_relay_binary_and_keep_shell() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-forward-test-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut peer = Peer::connect(routes, "ports-connection").await;
        for requested in [reserve_port().await, 0] {
            let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let expected = target.local_addr().unwrap();
            let cancel = CancelSignal::default();
            let port = peer.start_remote(requested, expected, cancel).await;
            assert_ne!(port, 0, "successful request must return a nonzero port");
            if requested != 0 {
                assert_eq!(port, requested);
            }
            assert!(manager.remote_routes.lock().unwrap().contains_key(&(
                "ports-connection".into(),
                "127.0.0.1".into(),
                u32::from(port)
            )));
            let payload = [0xde, 0x00, 0xad, 0xff, 0xbe];
            echo_remote(port, &target, &payload).await;
            peer.shell_echo(b"shell-still-alive").await;
            peer.stop_remote("127.0.0.1", port).await;
            assert!(!manager.remote_routes.lock().unwrap().contains_key(&(
                "ports-connection".into(),
                "127.0.0.1".into(),
                u32::from(port)
            )));
            assert!(timeout(LIMIT, TcpStream::connect(("127.0.0.1", port)))
                .await
                .unwrap()
                .is_err());
        }
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancel_closes_active_target_and_channel_without_stopping_another_route() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-cancel-test-{}", std::process::id())),
        );
        let mut peer = Peer::connect(manager.remote_routes.clone(), "cancel-connection").await;
        let target_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address_a = target_a.local_addr().unwrap();
        let target_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address_b = target_b.local_addr().unwrap();
        let port_a = peer
            .start_remote(reserve_port().await, address_a, CancelSignal::default())
            .await;
        let port_b = peer
            .start_remote(reserve_port().await, address_b, CancelSignal::default())
            .await;
        let mut remote_a = timeout(LIMIT, TcpStream::connect(("127.0.0.1", port_a)))
            .await
            .unwrap()
            .unwrap();
        let (mut accepted_a, _) = timeout(LIMIT, target_a.accept()).await.unwrap().unwrap();
        peer.stop_remote("127.0.0.1", port_a).await;
        let mut byte = [0];
        assert_eq!(
            timeout(LIMIT, accepted_a.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0,
            "cancel must close an already accepted target"
        );
        assert_eq!(
            timeout(LIMIT, remote_a.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0,
            "cancel must close the forwarded peer channel"
        );
        assert!(timeout(LIMIT, TcpStream::connect(("127.0.0.1", port_a)))
            .await
            .unwrap()
            .is_err());
        assert!(manager.remote_routes.lock().unwrap().contains_key(&(
            "cancel-connection".into(),
            "127.0.0.1".into(),
            u32::from(port_b)
        )));
        let payload = [0x91, 0x00, 0xfe, 0x03];
        echo_remote(port_b, &target_b, &payload).await;
        peer.shell_echo(b"shell-after-cancel").await;
        peer.stop_remote("127.0.0.1", port_b).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unknown_address_and_port_channels_close_without_target_connection() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-invalid-test-{}", std::process::id())),
        );
        let mut peer = Peer::connect(manager.remote_routes.clone(), "invalid-connection").await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let port = peer
            .start_remote(
                reserve_port().await,
                target_address,
                CancelSignal::default(),
            )
            .await;
        let wrong_port = if port == u16::MAX { port - 1 } else { port + 1 };
        assert_server_channel_closed(
            peer.open_forwarded("127.0.0.1", u32::from(wrong_port))
                .await,
        )
        .await;
        assert_server_channel_closed(peer.open_forwarded("localhost", u32::from(port)).await).await;
        assert_server_channel_closed(
            peer.open_forwarded("127.0.0.1", u32::from(port) + 10_000)
                .await,
        )
        .await;
        assert!(
            timeout(Duration::from_millis(250), target.accept())
                .await
                .is_err(),
            "unmatched wire metadata must not connect to any target"
        );
        echo_remote(port, &target, &[0x00, 0xca, 0xfe, 0xff]).await;
        peer.stop_remote("127.0.0.1", port).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_route_map_isolates_two_connections_with_identical_wire_metadata() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-owner-test-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut first = Peer::connect(routes.clone(), "connection-a").await;
        let mut second = Peer::connect(routes, "connection-b").await;
        let target_a = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_b = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let wire_address = "127.0.0.1";
        let wire_port = 43001u32;
        for (id, owner, target) in [
            ("route-a", "owner-a", target_a.local_addr().unwrap()),
            ("route-b", "owner-b", target_b.local_addr().unwrap()),
        ] {
            let route_info = SshForwardingInfo {
                id: id.into(),
                session_id: owner.into(),
                kind: SshForwardingType::Remote,
                bind_host: wire_address.into(),
                bind_port: wire_port as u16,
                target_address: target.ip().to_string(),
                target_port: target.port(),
                status: SshForwardingStatus::Active,
                last_error: None,
            };
            let runtime = ForwardingRuntime::new(route_info);
            let cancel = runtime.cancel.clone();
            manager
                .forwardings
                .lock()
                .unwrap()
                .insert(id.into(), runtime);
            let connection_id = if owner == "owner-a" {
                "connection-a"
            } else {
                "connection-b"
            };
            manager.remote_routes.lock().unwrap().insert(
                (connection_id.into(), wire_address.into(), wire_port),
                Arc::new(RemoteForwardRoute {
                    target_address: target.ip().to_string(),
                    target_port: target.port(),
                    cancel,
                    ready: None,
                }),
            );
        }
        let channel_a = first.open_forwarded(wire_address, wire_port).await;
        let channel_b = second.open_forwarded(wire_address, wire_port).await;
        let (mut target_socket_a, _) = timeout(LIMIT, target_a.accept()).await.unwrap().unwrap();
        let (mut target_socket_b, _) = timeout(LIMIT, target_b.accept()).await.unwrap().unwrap();
        let mut relay_a = channel_a.into_stream();
        let mut relay_b = channel_b.into_stream();
        let payload_a = [0xa1, 0, 0x10, 0xff];
        let payload_b = [0xb2, 0, 0x20, 0xfe];
        relay_a.write_all(&payload_a).await.unwrap();
        let mut received_a = [0; 4];
        timeout(LIMIT, target_socket_a.read_exact(&mut received_a))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received_a, payload_a);
        relay_b.write_all(&payload_b).await.unwrap();
        let mut received_b = [0; 4];
        timeout(LIMIT, target_socket_b.read_exact(&mut received_b))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received_b, payload_b);
        target_socket_a.write_all(&payload_a).await.unwrap();
        target_socket_b.write_all(&payload_b).await.unwrap();
        let mut echo_a = [0; 4];
        let mut echo_b = [0; 4];
        timeout(LIMIT, relay_a.read_exact(&mut echo_a))
            .await
            .unwrap()
            .unwrap();
        timeout(LIMIT, relay_b.read_exact(&mut echo_b))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(echo_a, payload_a);
        assert_eq!(echo_b, payload_b);

        manager.stop_forwardings_for_session("owner-a", "connection-a");
        let mut byte = [0];
        assert_eq!(
            timeout(LIMIT, relay_a.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(
            timeout(LIMIT, target_socket_a.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_server_channel_closed(first.open_forwarded(wire_address, wire_port).await).await;
        assert!(timeout(Duration::from_millis(250), target_a.accept())
            .await
            .is_err());

        let payload_b2 = [0xc3, 0, 0x30, 0xfd];
        relay_b.write_all(&payload_b2).await.unwrap();
        let mut received_b2 = [0; 4];
        timeout(LIMIT, target_socket_b.read_exact(&mut received_b2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received_b2, payload_b2);
        target_socket_b.write_all(&payload_b2).await.unwrap();
        let mut echo_b2 = [0; 4];
        timeout(LIMIT, relay_b.read_exact(&mut echo_b2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(echo_b2, payload_b2);
        first.shell_echo(b"owner-a-shell").await;
        second.shell_echo(b"owner-b-shell").await;
        manager.stop_forwardings_for_session("owner-b", "connection-b");
        first.cleanup().await;
        second.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn early_fixed_and_allocated_channels_wait_for_reply_then_relay_on_same_channel() {
    timeout(Duration::from_secs(30), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-eager-test-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut peer = Peer::connect(routes.clone(), "eager-connection").await;

        for requested_port in [reserve_port().await, 0] {
            let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let target_address = target.local_addr().unwrap();
            let (sender, receiver) = oneshot::channel();
            let cancel = CancelSignal::default();
            let wake = Arc::new(ReplyWake {
                ready: AtomicBool::new(false),
                notify: Notify::new(),
            });
            let mut sftp = None;
            let mut operation = Box::pin(super::handle_control(
                &mut peer.handle,
                &peer.writer,
                &mut sftp,
                "eager-connection",
                &routes,
                SshControl::StartRemoteForward {
                    bind_host: "127.0.0.1".into(),
                    bind_port: requested_port,
                    target_address: target_address.ip().to_string(),
                    target_port: target_address.port(),
                    cancel,
                    sender,
                },
            ));
            poll_control_once(operation.as_mut(), wake.clone());
            // A send can wake the first poll before the request reached the peer.
            // Require the peer's exact accepted address and requested/allocated port.
            wait_for_wake(&wake).await;
            let actual_port =
                wait_for_accepted_request(&mut peer.accepted_requests, "127.0.0.1", requested_port)
                    .await;
            assert_ne!(actual_port, 0);
            if requested_port != 0 {
                assert_eq!(actual_port, u32::from(requested_port));
            }

            let server_handle = peer.server_handle.clone();
            let mut early = timeout(
                LIMIT,
                server_handle.channel_open_forwarded_tcpip(
                    "127.0.0.1",
                    actual_port,
                    "127.0.0.1",
                    1234,
                ),
            )
            .await
            .unwrap()
            .unwrap();
            let channel_id = early.id();
            let wrong_port_channel = if requested_port == 0 {
                let wrong_port = if actual_port == u32::MAX {
                    actual_port - 1
                } else {
                    actual_port + 1
                };
                Some(
                    server_handle
                        .channel_open_forwarded_tcpip("127.0.0.1", wrong_port, "127.0.0.1", 1234)
                        .await
                        .unwrap(),
                )
            } else {
                None
            };

            if requested_port == 0 {
                let wrong = server_handle
                    .channel_open_forwarded_tcpip("localhost", actual_port, "127.0.0.1", 1234)
                    .await
                    .unwrap();
                let wrong_id = wrong.id();
                assert_server_channel_closed(wrong).await;
                wait_for_channel_close(&mut peer.close_events, &peer.state, wrong_id).await;
            }

            assert_channel_waits_without_target(&mut peer.close_events, &mut early, &target).await;
            assert!(!peer
                .state
                .closed_channels
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .contains(&channel_id));

            assert!(operation.as_mut().await);
            drop(operation);
            assert_eq!(receiver.await.unwrap().unwrap(), actual_port as u16);
            echo_on_channel(early, &target, &[0x51, 0x00, 0xe1, 0xff, 0x72]).await;
            if let Some(wrong_port_channel) = wrong_port_channel {
                let wrong_id = wrong_port_channel.id();
                assert_server_channel_closed(wrong_port_channel).await;
                wait_for_channel_close(&mut peer.close_events, &peer.state, wrong_id).await;
                assert!(timeout(Duration::from_millis(250), target.accept())
                    .await
                    .is_err());
            }
            assert!(routes.lock().unwrap().contains_key(&(
                "eager-connection".into(),
                "127.0.0.1".into(),
                actual_port
            )));

            echo_remote(actual_port as u16, &target, &[0x31, 0, 0xa4, 0xfe]).await;
            peer.shell_echo(b"after-early-channel").await;
            peer.stop_remote("127.0.0.1", actual_port as u16).await;
            assert!(!routes.lock().unwrap().contains_key(&(
                "eager-connection".into(),
                "127.0.0.1".into(),
                actual_port
            )));
        }
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropping_pending_control_closes_early_channel_without_connecting_target() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-drop-test-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut peer = Peer::connect(routes.clone(), "drop-connection").await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let requested_port = reserve_port().await;
        let (sender, _receiver) = oneshot::channel();
        let wake = Arc::new(ReplyWake {
            ready: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let mut sftp = None;
        let mut operation = Box::pin(super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut sftp,
            "drop-connection",
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: requested_port,
                target_address: target_address.ip().to_string(),
                target_port: target_address.port(),
                cancel: CancelSignal::default(),
                sender,
            },
        ));
        poll_control_once(operation.as_mut(), wake.clone());
        wait_for_wake(&wake).await;
        let actual_port =
            wait_for_accepted_request(&mut peer.accepted_requests, "127.0.0.1", requested_port)
                .await;
        let channel = peer
            .server_handle
            .channel_open_forwarded_tcpip("127.0.0.1", actual_port, "127.0.0.1", 1234)
            .await
            .unwrap();
        let channel_id = channel.id();
        let mut channel = channel;
        assert_channel_waits_without_target(&mut peer.close_events, &mut channel, &target).await;

        drop(operation);
        wait_for_channel_close(&mut peer.close_events, &peer.state, channel_id).await;
        assert!(timeout(Duration::from_millis(250), target.accept())
            .await
            .is_err());
        assert!(!routes.lock().unwrap().contains_key(&(
            "drop-connection".into(),
            "127.0.0.1".into(),
            actual_port
        )));

        peer.stop_remote("127.0.0.1", actual_port as u16).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rejected_request_closes_early_channel_without_connecting_target() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-reject-pending-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut peer = Peer::connect(routes.clone(), "reject-pending-connection").await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let requested_port = reserve_port().await;
        peer.state.reject_next.store(true, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        let wake = Arc::new(ReplyWake {
            ready: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let mut sftp = None;
        let mut operation = Box::pin(super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut sftp,
            "reject-pending-connection",
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: requested_port,
                target_address: target_address.ip().to_string(),
                target_port: target_address.port(),
                cancel: CancelSignal::default(),
                sender,
            },
        ));
        poll_control_once(operation.as_mut(), wake.clone());
        wait_for_wake(&wake).await;
        let (rejected_host, rejected_port) = timeout(LIMIT, peer.rejected_requests.recv())
            .await
            .expect("peer must decide the pending request")
            .expect("peer rejection observer must remain open");
        assert_eq!(rejected_host, "127.0.0.1");
        assert_eq!(rejected_port, u32::from(requested_port));

        let mut early = peer
            .server_handle
            .channel_open_forwarded_tcpip("127.0.0.1", rejected_port, "127.0.0.1", 1234)
            .await
            .unwrap();
        let channel_id = early.id();
        assert_channel_waits_without_target(&mut peer.close_events, &mut early, &target).await;

        assert!(timeout(LIMIT, operation.as_mut()).await.unwrap());
        drop(operation);
        assert!(
            receiver.await.unwrap().is_err(),
            "peer rejection must fail start"
        );
        wait_for_channel_close(&mut peer.close_events, &peer.state, channel_id).await;
        assert!(timeout(Duration::from_millis(250), target.accept())
            .await
            .is_err());
        assert!(!routes.lock().unwrap().contains_key(&(
            "reject-pending-connection".into(),
            "127.0.0.1".into(),
            rejected_port
        )));
        peer.shell_echo(b"rejected-request-keeps-shell").await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelling_pending_route_closes_early_channel_without_cleanup() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-cancel-pending-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let connection_id = "cancel-pending-connection";
        let session_id = "cancel-pending-session";
        let mut peer = Peer::connect(routes.clone(), connection_id).await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let requested_port = reserve_port().await;
        let runtime = ForwardingRuntime::new(SshForwardingInfo {
            id: "cancel-pending-route".into(),
            session_id: session_id.into(),
            kind: SshForwardingType::Remote,
            bind_host: "127.0.0.1".into(),
            bind_port: requested_port,
            target_address: target_address.ip().to_string(),
            target_port: target_address.port(),
            status: SshForwardingStatus::Active,
            last_error: None,
        });
        let cancel = runtime.cancel.clone();
        manager
            .forwardings
            .lock()
            .unwrap()
            .insert("cancel-pending-route".into(), runtime);
        let (sender, receiver) = oneshot::channel();
        let wake = Arc::new(ReplyWake {
            ready: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let mut sftp = None;
        let mut operation = Box::pin(super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut sftp,
            connection_id,
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: requested_port,
                target_address: target_address.ip().to_string(),
                target_port: target_address.port(),
                cancel: cancel.clone(),
                sender,
            },
        ));
        poll_control_once(operation.as_mut(), wake.clone());
        wait_for_wake(&wake).await;
        let actual_port =
            wait_for_accepted_request(&mut peer.accepted_requests, "127.0.0.1", requested_port)
                .await;
        let mut early = peer
            .server_handle
            .channel_open_forwarded_tcpip("127.0.0.1", actual_port, "127.0.0.1", 1234)
            .await
            .unwrap();
        let channel_id = early.id();
        assert_channel_waits_without_target(&mut peer.close_events, &mut early, &target).await;

        cancel.cancel();
        wait_for_channel_close(&mut peer.close_events, &peer.state, channel_id).await;
        assert!(timeout(Duration::from_millis(250), target.accept())
            .await
            .is_err());
        assert!(timeout(LIMIT, operation.as_mut()).await.unwrap());
        drop(operation);
        assert!(
            receiver.await.unwrap().is_err(),
            "cancelled route must fail start"
        );
        assert!(!routes.lock().unwrap().contains_key(&(
            connection_id.into(),
            "127.0.0.1".into(),
            actual_port
        )));
        assert!(manager
            .forwardings
            .lock()
            .unwrap()
            .contains_key("cancel-pending-route"));
        peer.stop_remote("127.0.0.1", actual_port as u16).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cleanup_during_pending_reply_does_not_republish_route() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-cleanup-pending-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let connection_id = "pending-cleanup-connection";
        let session_id = "pending-cleanup-session";
        let mut peer = Peer::connect(routes.clone(), connection_id).await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let requested_port = reserve_port().await;
        let runtime = ForwardingRuntime::new(SshForwardingInfo {
            id: "pending-cleanup-route".into(),
            session_id: session_id.into(),
            kind: SshForwardingType::Remote,
            bind_host: "127.0.0.1".into(),
            bind_port: requested_port,
            target_address: target_address.ip().to_string(),
            target_port: target_address.port(),
            status: SshForwardingStatus::Active,
            last_error: None,
        });
        let cancel = runtime.cancel.clone();
        manager
            .forwardings
            .lock()
            .unwrap()
            .insert("pending-cleanup-route".into(), runtime);
        let (sender, receiver) = oneshot::channel();
        let wake = Arc::new(ReplyWake {
            ready: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let mut sftp = None;
        let mut operation = Box::pin(super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut sftp,
            connection_id,
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: requested_port,
                target_address: target_address.ip().to_string(),
                target_port: target_address.port(),
                cancel,
                sender,
            },
        ));
        poll_control_once(operation.as_mut(), wake.clone());
        wait_for_wake(&wake).await;
        let actual_port =
            wait_for_accepted_request(&mut peer.accepted_requests, "127.0.0.1", requested_port)
                .await;
        let mut early = peer
            .server_handle
            .channel_open_forwarded_tcpip("127.0.0.1", actual_port, "127.0.0.1", 1234)
            .await
            .unwrap();
        let channel_id = early.id();
        assert_channel_waits_without_target(&mut peer.close_events, &mut early, &target).await;

        manager.stop_forwardings_for_session(session_id, connection_id);
        wait_for_channel_close(&mut peer.close_events, &peer.state, channel_id).await;
        assert!(timeout(Duration::from_millis(250), target.accept())
            .await
            .is_err());
        assert!(operation.as_mut().await);
        drop(operation);
        assert!(receiver.await.unwrap().is_err());
        assert!(!routes.lock().unwrap().contains_key(&(
            connection_id.into(),
            "127.0.0.1".into(),
            actual_port
        )));
        peer.stop_remote("127.0.0.1", actual_port as u16).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rejected_duplicate_fixed_request_preserves_original_route() {
    timeout(Duration::from_secs(15), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-duplicate-{}", std::process::id())),
        );
        let routes = manager.remote_routes.clone();
        let mut peer = Peer::connect(routes.clone(), "duplicate-connection").await;
        let port = reserve_port().await;
        let first_target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let first_address = first_target.local_addr().unwrap();
        let active_port = peer
            .start_remote(port, first_address, CancelSignal::default())
            .await;
        let second_target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let second_address = second_target.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        let keep = super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut None,
            "duplicate-connection",
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: port,
                target_address: second_address.ip().to_string(),
                target_port: second_address.port(),
                cancel: CancelSignal::default(),
                sender,
            },
        )
        .await;
        assert!(keep);
        assert!(
            receiver.await.unwrap().is_err(),
            "peer must reject the occupied fixed port"
        );
        let original = routes
            .lock()
            .unwrap()
            .get(&(
                "duplicate-connection".into(),
                "127.0.0.1".into(),
                u32::from(active_port),
            ))
            .cloned()
            .expect("the first active route must remain installed");
        assert_eq!(original.target_address, first_address.ip().to_string());
        assert_eq!(original.target_port, first_address.port());
        assert!(timeout(Duration::from_millis(250), second_target.accept())
            .await
            .is_err());
        echo_remote(active_port, &first_target, &[0xd1, 0, 0x41, 0xff]).await;
        peer.stop_remote("127.0.0.1", active_port).await;
        peer.cleanup().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn normal_eager_peer_completes_32_binary_echoes_without_rejections() {
    timeout(Duration::from_secs(60), async {
        let manager = SshManager::new(
            std::env::temp_dir().join(format!("remote-eager-32-{}", std::process::id())),
        );
        let mut peer = Peer::connect(manager.remote_routes.clone(), "eager-32-connection").await;
        peer.state.auto_eager.store(true, Ordering::Relaxed);

        for attempt in 0..32 {
            let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let target_address = target.local_addr().unwrap();
            let port = peer
                .start_remote(
                    reserve_port().await,
                    target_address,
                    CancelSignal::default(),
                )
                .await;
            let (mut target_socket, _) = timeout(LIMIT, target.accept()).await.unwrap().unwrap();
            let mut payload = [0; 4];
            timeout(LIMIT, target_socket.read_exact(&mut payload))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(payload, [0x51, 0, 0xe1, 0xff]);
            target_socket.write_all(&payload).await.unwrap();
            let (channel_id, accepted) = wait_for_auto_result(&peer, attempt).await;
            assert!(
                accepted,
                "eager channel {channel_id:?} rejected on attempt {attempt}"
            );
            echo_remote(port, &target, &[attempt as u8, 0, 0xab, 0xfe]).await;
            peer.shell_echo(b"normal-eager-shell").await;
            peer.stop_remote("127.0.0.1", port).await;
        }

        let results = peer
            .state
            .auto_results
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        assert_eq!(results.len(), 32);
        assert!(results.iter().all(|(_, accepted)| *accepted));
        peer.cleanup().await;
    })
    .await
    .unwrap();
}
