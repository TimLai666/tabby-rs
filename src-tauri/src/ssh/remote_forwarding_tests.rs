use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{client, server, Channel, ChannelId, ChannelMsg, CryptoVec};
use tokio::{
    io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
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

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
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

async fn assert_server_channel_closed(channel: Channel<server::Msg>) {
    let mut stream = channel.into_stream();
    let mut byte = [0];
    let result = timeout(LIMIT, stream.read(&mut byte)).await.unwrap();
    assert!(
        result.map(|count| count == 0).unwrap_or(true),
        "unknown route channel must close"
    );
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
