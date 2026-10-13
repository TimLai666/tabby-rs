use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use russh::{client, server, Channel, ChannelId, ChannelMsg, CryptoVec};
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
    pending::CancelSignal,
    SshControl,
};

const LIMIT: Duration = Duration::from_secs(5);
const BADGE: &str = "\x1b[41m\x1b[30m X \x1b[39m\x1b[49m";
type Messages = Arc<Mutex<Vec<String>>>;

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
    async fn finish(mut self) -> T {
        let task = self.0.as_mut().unwrap();
        match timeout(LIMIT, task).await {
            Ok(Ok(value)) => {
                self.0.take();
                value
            }
            Ok(Err(error)) => panic!("task failed: {error}"),
            Err(_) => {
                if let Some(task) = self.0.take() {
                    task.abort();
                    let _ = task.await;
                }
                panic!("task exceeded {LIMIT:?}");
            }
        }
    }
}

fn info(kind: SshForwardingType) -> SshForwardingInfo {
    SshForwardingInfo {
        id: "forward-test".into(),
        session_id: "owner-test".into(),
        kind,
        bind_host: "127.0.0.1".into(),
        bind_port: 0,
        target_address: "configured-target.invalid".into(),
        target_port: 65000,
        status: SshForwardingStatus::Active,
        last_error: None,
    }
}

struct Peer {
    reject: Arc<Mutex<bool>>,
    gate: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
    requested: mpsc::UnboundedSender<(String, u32)>,
}
impl server::Handler for Peer {
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
    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(id)
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        _: Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        _: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let _ = self.requested.send((host.to_owned(), port));
        let gate = { self.gate.lock().unwrap().take() };
        if let Some(gate) = gate {
            let _ = timeout(LIMIT, gate).await;
        }
        Ok(!*self.reject.lock().unwrap())
    }
    async fn channel_eof(
        &mut self,
        id: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        session.eof(id)
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

struct SshPeer {
    handle: client::Handle<Pinned>,
    server: AbortTask<()>,
    requested: mpsc::UnboundedReceiver<(String, u32)>,
    reject: Arc<Mutex<bool>>,
}
impl SshPeer {
    async fn new(reject: bool, gate: Option<oneshot::Receiver<()>>) -> Self {
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
        let reject = Arc::new(Mutex::new(reject));
        let server_reject = reject.clone();
        let gate = Arc::new(Mutex::new(gate));
        let server_gate = gate.clone();
        let (request_tx, requested) = mpsc::unbounded_channel();
        // Keep the server alive until cleanup disconnects the SSH client.
        let server = spawn(async move {
            let (tcp, _) = timeout(LIMIT, listener.accept()).await.unwrap().unwrap();
            if let Ok(session) = server::run_stream(
                Arc::new(config),
                tcp,
                Peer {
                    reject: server_reject,
                    gate: server_gate,
                    requested: request_tx,
                },
            )
            .await
            {
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
        assert!(timeout(LIMIT, handle.authenticate_none("test"))
            .await
            .unwrap()
            .unwrap()
            .success());
        timeout(LIMIT, handle.channel_open_session())
            .await
            .unwrap()
            .unwrap();
        Self {
            handle,
            server,
            requested,
            reject,
        }
    }
    async fn cleanup(self) {
        let _ = timeout(
            LIMIT,
            self.handle
                .disconnect(russh::Disconnect::ByApplication, "done", "en"),
        )
        .await;
        let _ = self.server.finish().await;
    }
    // Lets a spawned control task own the peer while the test observes server requests.
    fn detach_requests(&mut self) -> mpsc::UnboundedReceiver<(String, u32)> {
        let (_, empty) = mpsc::unbounded_channel();
        std::mem::replace(&mut self.requested, empty)
    }
    async fn native_rejection(&mut self, host: &str, port: u16) -> String {
        timeout(
            LIMIT,
            self.handle
                .channel_open_direct_tcpip(host, port.into(), "127.0.0.1", 0),
        )
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
    }
    async fn control(&mut self, command: SshControl) -> bool {
        let (_reader, writer) = timeout(LIMIT, self.handle.channel_open_session())
            .await
            .unwrap()
            .unwrap()
            .split();
        handle_control_bounded(&mut self.handle, &writer, command).await
    }
    async fn shell_echo(&mut self, payload: &[u8]) {
        let mut shell = timeout(LIMIT, self.handle.channel_open_session())
            .await
            .unwrap()
            .unwrap();
        timeout(LIMIT, shell.request_shell(false))
            .await
            .unwrap()
            .unwrap();
        timeout(LIMIT, shell.data(payload)).await.unwrap().unwrap();
        match timeout(LIMIT, shell.wait()).await.unwrap().unwrap() {
            ChannelMsg::Data { data } => assert_eq!(&data[..], payload),
            other => panic!("expected shell echo, got {other:?}"),
        }
    }
}

async fn handle_control_bounded<H: client::Handler>(
    handle: &mut client::Handle<H>,
    writer: &russh::ChannelWriteHalf<client::Msg>,
    control: SshControl,
) -> bool {
    let routes = Arc::new(Mutex::new(Default::default()));
    let mut sftp = None;
    timeout(
        LIMIT,
        super::handle_control(
            handle,
            writer,
            &mut sftp,
            "connection-test",
            &routes,
            control,
        ),
    )
    .await
    .expect("handle_control exceeded deadline")
}

async fn run_local(
    item: SshForwardingInfo,
    cancel: CancelSignal,
    messages: Messages,
) -> (
    std::net::SocketAddr,
    mpsc::Receiver<SshControl>,
    AbortTask<()>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel(8);
    let task = spawn(forwarding::run_local_forward(
        item,
        listener,
        tx,
        cancel,
        move |message| messages.lock().unwrap().push(message),
    ));
    (addr, rx, task)
}

async fn write_bounded(stream: &mut TcpStream, bytes: &[u8]) {
    timeout(LIMIT, stream.write_all(bytes))
        .await
        .expect("socket write exceeded deadline")
        .unwrap();
}

async fn open_local_peer(
    kind: SshForwardingType,
    address: std::net::SocketAddr,
    host: &str,
    port: u16,
) -> TcpStream {
    let mut socket = timeout(LIMIT, TcpStream::connect(address))
        .await
        .unwrap()
        .unwrap();
    if kind == SshForwardingType::Dynamic {
        write_bounded(&mut socket, &[5, 1, 0]).await;
        let mut greeting = [0; 2];
        timeout(LIMIT, socket.read_exact(&mut greeting))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(greeting, [5, 0]);
        let domain = host.as_bytes();
        let mut request = vec![5, 1, 0, 3, domain.len() as u8];
        request.extend_from_slice(domain);
        request.extend_from_slice(&port.to_be_bytes());
        write_bounded(&mut socket, &request).await;
    }
    socket
}

async fn assert_eof(socket: &mut TcpStream) {
    let mut byte = [0];
    assert_eq!(
        timeout(LIMIT, socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

// SOCKS5 "connection not allowed by ruleset": the upstream denial is exactly two bytes.
async fn assert_socks_denial(socket: &mut TcpStream) {
    let mut denial = [0; 2];
    timeout(LIMIT, socket.read_exact(&mut denial))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(denial, [5, 2]);
}

// Only meaningful after the real SSH request was observed; the 100ms window is
// the observation period in which no application response may appear.
async fn assert_no_client_bytes(socket: &mut TcpStream) {
    let mut byte = [0];
    assert!(
        timeout(Duration::from_millis(100), socket.read(&mut byte))
            .await
            .is_err(),
        "client received bytes before the SSH result"
    );
}

struct CancelOnDrop(CancelSignal);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn spawn_control(mut peer: SshPeer, command: SshControl) -> AbortTask<(SshPeer, bool)> {
    spawn(async move {
        let keep = peer.control(command).await;
        (peer, keep)
    })
}

async fn rejection_and_recovery(kind: SshForwardingType, requested_host: &str, port: u16) {
    let mut peer = SshPeer::new(true, None).await;
    let native = peer.native_rejection(requested_host, port).await;
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        (requested_host.to_owned(), u32::from(port))
    );
    let mut item = info(kind);
    if kind == SshForwardingType::Local {
        item.target_address = requested_host.into();
        item.target_port = port;
    }
    let cancel = CancelSignal::default();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let (addr, mut controls, task) = run_local(item, cancel.clone(), messages.clone()).await;

    let mut socket = open_local_peer(kind, addr, requested_host, port).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        (requested_host.to_owned(), u32::from(port))
    );
    if kind == SshForwardingType::Dynamic {
        assert_socks_denial(&mut socket).await;
    }
    assert_eof(&mut socket).await;
    let diagnostic = messages.lock().unwrap().clone();
    assert_eq!(diagnostic.len(), 1);
    let expected_description = match kind {
        SshForwardingType::Local => {
            format!("(local) 127.0.0.1:0 → (remote) {requested_host}:{port}")
        }
        SshForwardingType::Dynamic => "(dynamic) 127.0.0.1:0".to_owned(),
        _ => unreachable!(),
    };
    assert_eq!(
        diagnostic[0],
        format!(
            "{BADGE} Remote has rejected the forwarded connection to {requested_host}:{port} via {expected_description}: {native}"
        )
    );

    // A later real SSH channel on this same listener must work and stay quiet.
    *peer.reject.lock().unwrap() = false;
    let mut recovered = open_local_peer(kind, addr, requested_host, port).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        (requested_host.to_owned(), u32::from(port))
    );
    if kind == SshForwardingType::Dynamic {
        let mut socks_reply = [0; 10];
        timeout(LIMIT, recovered.read_exact(&mut socks_reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(socks_reply, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    }
    let payload = [0x11, 0, 0xfe, 0x80, 0x42];
    write_bounded(&mut recovered, &payload).await;
    let mut echoed = [0; 5];
    timeout(LIMIT, recovered.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, payload);
    peer.shell_echo(&[0, 0xff, 0x41, 0x80]).await;
    assert_eq!(messages.lock().unwrap().len(), 1);

    drop(recovered);
    drop(socket);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn local_rejection_recovers_and_keeps_shell_usable() {
    rejection_and_recovery(SshForwardingType::Local, "127.0.0.1", 9).await;
}

#[tokio::test]
async fn dynamic_rejection_recovers_and_keeps_shell_usable() {
    rejection_and_recovery(SshForwardingType::Dynamic, "requested.example", 2209).await;
}

#[tokio::test]
async fn successful_local_relay_and_eof_are_quiet() {
    let mut peer = SshPeer::new(false, None).await;
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Local),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = open_local_peer(SshForwardingType::Local, addr, "127.0.0.1", 9).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    let payload = [0xde, 0x00, 0xad, 0xff, 0xbe];
    write_bounded(&mut socket, &payload).await;
    let mut echoed = [0; 5];
    timeout(LIMIT, socket.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, payload);
    timeout(LIMIT, socket.shutdown())
        .await
        .expect("socket shutdown exceeded deadline")
        .unwrap();
    assert_eof(&mut socket).await;
    peer.shell_echo(b"shell still works").await;
    assert!(messages.lock().unwrap().is_empty());
    drop(socket);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn cancelled_pending_rejection_is_silent_and_closes_local_socket() {
    let (release, gate) = oneshot::channel();
    let mut peer = SshPeer::new(true, Some(gate)).await;
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Local),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    let (reader, writer) = timeout(LIMIT, peer.handle.channel_open_session())
        .await
        .unwrap()
        .unwrap()
        .split();
    let (dummy_tx, dummy_rx) = mpsc::unbounded_channel();
    let mut requested = std::mem::replace(&mut peer.requested, dummy_rx);
    drop(dummy_tx);
    let control_task = spawn(async move {
        let routes = Arc::new(Mutex::new(Default::default()));
        let mut sftp = None;
        let keep = timeout(
            LIMIT,
            super::handle_control(
                &mut peer.handle,
                &writer,
                &mut sftp,
                "connection-test",
                &routes,
                command,
            ),
        )
        .await
        .expect("handle_control exceeded deadline");
        (peer, keep)
    });
    let (_host, _port) = timeout(LIMIT, requested.recv()).await.unwrap().unwrap();
    cancel.cancel();
    assert_eof(&mut socket).await;
    release.send(()).unwrap();
    let (peer, keep) = control_task.finish().await;
    assert!(keep);
    assert!(messages.lock().unwrap().is_empty());
    drop(reader);
    drop(socket);
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn two_local_listener_callbacks_are_isolated() {
    let mut peer_a = SshPeer::new(true, None).await;
    let mut peer_b = SshPeer::new(true, None).await;
    let a = Arc::new(Mutex::new(Vec::new()));
    let b = Arc::new(Mutex::new(Vec::new()));
    let cancel_a = CancelSignal::default();
    let cancel_b = CancelSignal::default();
    let mut item_a = info(SshForwardingType::Local);
    item_a.target_address = "a.example".into();
    item_a.target_port = 9;
    let mut item_b = info(SshForwardingType::Local);
    item_b.target_address = "b.example".into();
    item_b.target_port = 9;
    let (addr_a, mut controls_a, task_a) = run_local(item_a, cancel_a.clone(), a.clone()).await;
    let (addr_b, mut controls_b, task_b) = run_local(item_b, cancel_b.clone(), b.clone()).await;
    let mut socket_a = open_local_peer(SshForwardingType::Local, addr_a, "a.example", 9).await;
    let mut socket_b = open_local_peer(SshForwardingType::Local, addr_b, "b.example", 9).await;
    let command_a = timeout(LIMIT, controls_a.recv()).await.unwrap().unwrap();
    let command_b = timeout(LIMIT, controls_b.recv()).await.unwrap().unwrap();
    assert!(peer_a.control(command_a).await);
    assert!(peer_b.control(command_b).await);
    assert_eof(&mut socket_a).await;
    assert_eof(&mut socket_b).await;
    let message_a = a.lock().unwrap().clone();
    let message_b = b.lock().unwrap().clone();
    assert_eq!(message_a.len(), 1);
    assert_eq!(message_b.len(), 1);
    assert!(message_a[0].contains("a.example:9"));
    assert!(!message_a[0].contains("b.example:9"));
    assert!(message_b[0].contains("b.example:9"));
    assert!(!message_b[0].contains("a.example:9"));
    cancel_a.cancel();
    cancel_b.cancel();
    drop(socket_a);
    drop(socket_b);
    drop(controls_a);
    drop(controls_b);
    task_a.finish().await;
    task_b.finish().await;
    peer_a.cleanup().await;
    peer_b.cleanup().await;
}

#[tokio::test]
async fn dynamic_hostile_controls_are_escaped_and_unicode_is_kept() {
    let requested = "host\u{1b}\n猫.example";
    let mut peer = SshPeer::new(true, None).await;
    let native = peer.native_rejection(requested, 9).await;
    let _ = timeout(LIMIT, peer.requested.recv())
        .await
        .unwrap()
        .unwrap();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Dynamic),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = open_local_peer(SshForwardingType::Dynamic, addr, requested, 9).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        (requested.to_owned(), 9)
    );
    assert_socks_denial(&mut socket).await;
    assert_eof(&mut socket).await;
    let result = messages.lock().unwrap().clone();
    assert_eq!(result.len(), 1);
    assert!(result[0].starts_with(BADGE));
    let diagnostic = &result[0][BADGE.len()..];
    assert!(diagnostic.contains("host\\u{1b}\\n猫.example:9"));
    assert!(diagnostic.contains("via (dynamic) 127.0.0.1:0"));
    assert!(diagnostic.ends_with(&native));
    assert!(!diagnostic.contains('\u{1b}'));
    assert!(!diagnostic.contains('\n'));
    drop(socket);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn dynamic_success_waits_for_ssh_approval_and_orders_binary() {
    let host = "approved.example";
    let (release, gate) = oneshot::channel();
    let mut peer = SshPeer::new(false, Some(gate)).await;
    let mut requested = peer.detach_requests();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let _cancel_guard = CancelOnDrop(cancel.clone());
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Dynamic),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = open_local_peer(SshForwardingType::Dynamic, addr, host, 2209).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    let control = spawn_control(peer, command);
    assert_eq!(
        timeout(LIMIT, requested.recv()).await.unwrap().unwrap(),
        (host.to_owned(), 2209)
    );
    // Non-text bytes pipelined right after CONNECT must wait behind the SSH result.
    let payload = [0x00, 0xff, 0x80, 0x0d, 0x0a, 0x1b];
    write_bounded(&mut socket, &payload).await;
    assert_no_client_bytes(&mut socket).await;
    assert!(messages.lock().unwrap().is_empty());

    release.send(()).unwrap();
    let (mut peer, keep) = control.finish().await;
    assert!(keep);
    let mut success = [0; 10];
    timeout(LIMIT, socket.read_exact(&mut success))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(success, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    let mut echoed = [0; 6];
    timeout(LIMIT, socket.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, payload);
    timeout(LIMIT, socket.shutdown())
        .await
        .expect("socket shutdown exceeded deadline")
        .unwrap();
    assert_eof(&mut socket).await;
    peer.shell_echo(b"shell still works").await;
    assert!(messages.lock().unwrap().is_empty());

    drop(socket);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn dynamic_rejection_waits_for_ssh_approval() {
    let host = "requested.example";
    let (release, gate) = oneshot::channel();
    let mut peer = SshPeer::new(true, Some(gate)).await;
    let mut requested = peer.detach_requests();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let _cancel_guard = CancelOnDrop(cancel.clone());
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Dynamic),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = open_local_peer(SshForwardingType::Dynamic, addr, host, 2209).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    let control = spawn_control(peer, command);
    assert_eq!(
        timeout(LIMIT, requested.recv()).await.unwrap().unwrap(),
        (host.to_owned(), 2209)
    );
    assert_no_client_bytes(&mut socket).await;
    assert!(messages.lock().unwrap().is_empty());

    release.send(()).unwrap();
    let (mut peer, keep) = control.finish().await;
    assert!(keep);
    assert_socks_denial(&mut socket).await;
    assert_eof(&mut socket).await;
    // The gate was consumed by the forwarded request, so this probe is rejected immediately.
    let native = peer.native_rejection(host, 2209).await;
    assert_eq!(
        timeout(LIMIT, requested.recv()).await.unwrap().unwrap(),
        (host.to_owned(), 2209)
    );
    assert_eq!(
        messages.lock().unwrap().clone(),
        vec![format!(
            "{BADGE} Remote has rejected the forwarded connection to {host}:2209 via (dynamic) 127.0.0.1:0: {native}"
        )]
    );

    // Same listener recovers after the held rejection.
    *peer.reject.lock().unwrap() = false;
    let mut recovered = open_local_peer(SshForwardingType::Dynamic, addr, host, 2209).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, requested.recv()).await.unwrap().unwrap(),
        (host.to_owned(), 2209)
    );
    let mut success = [0; 10];
    timeout(LIMIT, recovered.read_exact(&mut success))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(success, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    let payload = [0x22, 0, 0xfd];
    write_bounded(&mut recovered, &payload).await;
    let mut echoed = [0; 3];
    timeout(LIMIT, recovered.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, payload);
    peer.shell_echo(b"shell after rejection").await;
    assert_eq!(messages.lock().unwrap().len(), 1);

    drop(recovered);
    drop(socket);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

async fn cancelled_pending_dynamic_is_silent(reject: bool) {
    let host = "late.example";
    let (release, gate) = oneshot::channel();
    let mut peer = SshPeer::new(reject, Some(gate)).await;
    let mut requested = peer.detach_requests();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let cancel = CancelSignal::default();
    let _cancel_guard = CancelOnDrop(cancel.clone());
    let (addr, mut controls, task) = run_local(
        info(SshForwardingType::Dynamic),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    let mut socket = open_local_peer(SshForwardingType::Dynamic, addr, host, 9).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    let control = spawn_control(peer, command);
    assert_eq!(
        timeout(LIMIT, requested.recv()).await.unwrap().unwrap(),
        (host.to_owned(), 9)
    );

    // Cancellation must close the client without waiting for the held SSH result.
    cancel.cancel();
    assert_eof(&mut socket).await;
    release.send(()).unwrap();
    let (mut peer, keep) = control.finish().await;
    assert!(keep);
    assert!(messages.lock().unwrap().is_empty());
    peer.shell_echo(b"shell after late result").await;
    assert!(messages.lock().unwrap().is_empty());

    drop(socket);
    drop(controls);
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn cancelled_pending_dynamic_is_silent_for_late_approval_and_rejection() {
    cancelled_pending_dynamic_is_silent(false).await;
    cancelled_pending_dynamic_is_silent(true).await;
}

async fn socks_parser_listener() -> (
    std::net::SocketAddr,
    mpsc::Receiver<SshControl>,
    CancelSignal,
    Messages,
    AbortTask<()>,
) {
    let cancel = CancelSignal::default();
    let messages = Arc::new(Mutex::new(Vec::new()));
    let (addr, controls, task) = run_local(
        info(SshForwardingType::Dynamic),
        cancel.clone(),
        messages.clone(),
    )
    .await;
    (addr, controls, cancel, messages, task)
}

async fn socks_parser_connect(addr: std::net::SocketAddr, request: &[u8]) -> TcpStream {
    let mut socket = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut socket, &[5, 1, 0]).await;
    let mut greeting = [0; 2];
    timeout(LIMIT, socket.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(greeting, [5, 0]);
    write_bounded(&mut socket, request).await;
    socket
}

#[tokio::test]
async fn socks_parser_greeting_errors_and_method_limits() {
    let mut peer = SshPeer::new(true, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;
    for greeting in [&[4][..], &[4, 1, 0][..], &[5, 0][..]] {
        let mut socket = timeout(LIMIT, TcpStream::connect(addr))
            .await
            .unwrap()
            .unwrap();
        write_bounded(&mut socket, greeting).await;
        assert_eof(&mut socket).await;
        drop(socket);
    }
    assert!(controls.try_recv().is_err());
    assert!(messages.lock().unwrap().is_empty());
    let mut unsupported = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(
        &mut unsupported,
        &[
            5, 2, 1, 2, 5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1, 0, 0xff, 0x80,
        ],
    )
    .await;
    let mut response = [0; 2];
    timeout(LIMIT, unsupported.read_exact(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response, [5, 0xff]);
    assert_eof(&mut unsupported).await;

    // Both boundary-sized method lists contain no-auth as their final method.
    for count in [33usize, 255] {
        let mut socket = timeout(LIMIT, TcpStream::connect(addr))
            .await
            .unwrap()
            .unwrap();
        let mut greeting = vec![5, count as u8];
        greeting.resize(count + 2, 2);
        *greeting.last_mut().unwrap() = 0;
        write_bounded(&mut socket, &greeting).await;
        let mut selected = [0; 2];
        timeout(LIMIT, socket.read_exact(&mut selected))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected, [5, 0]);
        write_bounded(&mut socket, &[5, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1]).await;
        let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
        assert!(peer.control(command).await);
        assert_eq!(
            timeout(LIMIT, peer.requested.recv())
                .await
                .unwrap()
                .unwrap(),
            ("127.0.0.1".into(), 2209)
        );
        let mut denial = [0; 2];
        timeout(LIMIT, socket.read_exact(&mut denial))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(denial, [5, 2]);
        assert_eof(&mut socket).await;
        drop(socket);
    }
    assert!(controls.try_recv().is_err());
    assert_eq!(messages.lock().unwrap().len(), 2);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn socks_parser_complete_bind_udp_and_invalid_headers_close_without_ssh() {
    let mut peer = SshPeer::new(true, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;
    let cases: &[(&str, &[u8], Option<[u8; 2]>)] = &[
        (
            "bind-ipv4",
            &[5, 2, 0, 1, 127, 0, 0, 1, 8, 0xa1],
            Some([5, 7]),
        ),
        (
            "udp-ipv4",
            &[5, 3, 0, 1, 127, 0, 0, 1, 8, 0xa1],
            Some([5, 7]),
        ),
        (
            "bind-domain",
            &[5, 2, 0, 3, 4, b't', b'e', b's', b't', 8, 0xa1],
            Some([5, 7]),
        ),
        (
            "udp-ipv6",
            &[
                5, 3, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 8, 0xa1,
            ],
            Some([5, 7]),
        ),
        ("wrong-version", &[4, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1], None),
        (
            "unknown-command",
            &[5, 4, 0, 1, 127, 0, 0, 1, 8, 0xa1],
            None,
        ),
        ("unknown-atyp", &[5, 1, 0, 2, 127, 0, 0, 1, 8, 0xa1], None),
    ];
    for (name, request, first_reply) in cases {
        let mut socket = socks_parser_connect(addr, request).await;
        if let Some(expected) = first_reply {
            let mut reply = [0; 2];
            timeout(LIMIT, socket.read_exact(&mut reply))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&reply, expected, "{name}");
        }
        assert_eof(&mut socket).await;
        assert!(controls.try_recv().is_err(), "{name} reached SSH control");
        assert!(
            messages.lock().unwrap().is_empty(),
            "{name} emitted diagnostic"
        );
        drop(socket);
    }

    // Header-only invalid requests and complete pipelined tails have the same wire result.
    for request in [
        &[5, 4][..],
        &[5, 4, 0, 1, 127, 0, 0, 1, 8, 0xa1, 0x41, 0x42][..],
        &[4][..],
        &[4, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1, 0x41][..],
        &[5, 1, 0, 2][..],
        &[5, 1, 0, 2, 127, 0, 0, 1, 8, 0xa1, 0x41, 0x42][..],
    ] {
        let mut socket = socks_parser_connect(addr, request).await;
        assert_eof(&mut socket).await;
        assert!(controls.try_recv().is_err());
        assert!(messages.lock().unwrap().is_empty());
        drop(socket);
    }
    let mut truncated = socks_parser_connect(addr, &[5, 1, 0, 1, 127]).await;
    timeout(LIMIT, truncated.shutdown()).await.unwrap().unwrap();
    assert_eof(&mut truncated).await;
    assert!(controls.try_recv().is_err());
    assert!(messages.lock().unwrap().is_empty());
    peer.shell_echo(b"parser-rejection-keeps-ssh-alive").await;
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn socks_parser_connect_metadata_matches_compatibility_inputs() {
    let mut peer = SshPeer::new(true, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;
    let cases: &[(&[u8], &str, u32)] = &[
        (&[5, 1, 9, 1, 127, 0, 0, 1, 8, 0xa1], "127.0.0.1", 2209),
        (&[5, 1, 0, 1, 127, 0, 0, 1, 0, 0], "127.0.0.1", 0),
        (&[5, 1, 0, 3, 0, 8, 0xa1], "", 2209),
        (&[5, 1, 0, 3, 2, 0xff, b'a', 8, 0xa1], "�a", 2209),
        (
            &[
                5, 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 8, 0xa1,
            ],
            "0000:0000:0000:0000:0000:0000:0000:0001",
            2209,
        ),
    ];
    for (request, expected_host, expected_port) in cases {
        let mut socket = socks_parser_connect(addr, request).await;
        let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
        assert!(peer.control(command).await);
        assert_eq!(
            timeout(LIMIT, peer.requested.recv())
                .await
                .unwrap()
                .unwrap(),
            ((*expected_host).into(), *expected_port)
        );
        let mut denial = [0; 2];
        timeout(LIMIT, socket.read_exact(&mut denial))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(denial, [5, 2]);
        assert_eof(&mut socket).await;
        drop(socket);
    }
    assert_eq!(messages.lock().unwrap().len(), cases.len());
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn socks_parser_pipelined_invalid_then_valid_reuses_listener() {
    let mut peer = SshPeer::new(false, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;
    let mut invalid =
        socks_parser_connect(addr, &[5, 4, 0, 1, 127, 0, 0, 1, 8, 0xa1, 0, 0xff, 0x80]).await;
    assert_eof(&mut invalid).await;
    drop(invalid);
    assert!(controls.try_recv().is_err());
    assert!(messages.lock().unwrap().is_empty());

    let mut valid = socks_parser_connect(addr, &[5, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1]).await;
    write_bounded(&mut valid, &[0, 0xff, 0x80, 0x42]).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        ("127.0.0.1".into(), 2209)
    );
    let mut success = [0; 10];
    timeout(LIMIT, valid.read_exact(&mut success))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(success, [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    let mut echoed = [0; 4];
    timeout(LIMIT, valid.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed, [0, 0xff, 0x80, 0x42]);
    timeout(LIMIT, valid.shutdown()).await.unwrap().unwrap();
    assert_eof(&mut valid).await;
    assert!(messages.lock().unwrap().is_empty());
    drop(valid);
    drop(controls);
    cancel.cancel();
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn socks_parser_incomplete_bind_waits_and_cancellation_closes_write_held_client() {
    let mut peer = SshPeer::new(true, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;

    let mut bind = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut bind, &[5, 1, 0]).await;
    let mut greeting = [0; 2];
    timeout(LIMIT, bind.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(greeting, [5, 0]);
    write_bounded(&mut bind, &[5, 2, 0, 1, 127, 0, 0, 1, 8]).await;
    assert!(
        timeout(Duration::from_millis(200), async {
            let mut byte = [0];
            bind.read(&mut byte).await
        })
        .await
        .is_err(),
        "incomplete BIND received a premature response"
    );
    assert!(controls.try_recv().is_err());
    assert!(messages.lock().unwrap().is_empty());
    write_bounded(&mut bind, &[0xa1]).await;
    let mut refusal = [0; 2];
    timeout(LIMIT, bind.read_exact(&mut refusal))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refusal, [5, 7]);
    assert_eof(&mut bind).await;
    // Keep the rejected client write half open until cancellation.

    // A second incomplete BIND remains open until listener cancellation.
    let mut held_bind = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut held_bind, &[5, 1, 0]).await;
    timeout(LIMIT, held_bind.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut held_bind, &[5, 2, 0, 1, 127, 0, 0, 1, 8]).await;
    assert!(
        timeout(Duration::from_millis(200), async {
            let mut byte = [0];
            held_bind.read(&mut byte).await
        })
        .await
        .is_err(),
        "held incomplete BIND received a premature response"
    );

    // Pipeline application bytes behind a valid CONNECT that SSH rejects.
    let mut denied = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut denied, &[5, 1, 0]).await;
    timeout(LIMIT, denied.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    let mut request_and_tail = vec![5, 1, 0, 1, 127, 0, 0, 1, 8, 0xa1];
    request_and_tail.extend_from_slice(&[0, 0xff, 0x80, 0x42]);
    write_bounded(&mut denied, &request_and_tail).await;
    let command = timeout(LIMIT, controls.recv()).await.unwrap().unwrap();
    assert!(peer.control(command).await);
    assert_eq!(
        timeout(LIMIT, peer.requested.recv())
            .await
            .unwrap()
            .unwrap(),
        ("127.0.0.1".into(), 2209)
    );
    let mut denial = [0; 2];
    timeout(LIMIT, denied.read_exact(&mut denial))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(denial, [5, 2]);
    assert_eof(&mut denied).await;
    assert_eq!(messages.lock().unwrap().len(), 1);

    cancel.cancel();
    assert_eof(&mut held_bind).await;
    for socket in [&mut bind, &mut held_bind, &mut denied] {
        let write_side_closed = timeout(LIMIT, async {
            let buffer = [0xa5; 1024];
            loop {
                match socket.write(&buffer).await {
                    Ok(0) | Err(_) => break true,
                    Ok(_) => {}
                }
            }
        })
        .await;
        assert!(
            write_side_closed.is_ok(),
            "cancelled task kept draining client writes"
        );
    }
    assert!(controls.try_recv().is_err());
    drop(bind);
    drop(held_bind);
    drop(denied);
    drop(controls);
    task.finish().await;
    peer.cleanup().await;
}

#[tokio::test]
async fn socks_parser_incomplete_connect_has_no_native_30_second_deadline() {
    let mut peer = SshPeer::new(true, None).await;
    let (addr, mut controls, cancel, messages, task) = socks_parser_listener().await;
    let mut socket = timeout(LIMIT, TcpStream::connect(addr))
        .await
        .unwrap()
        .unwrap();
    write_bounded(&mut socket, &[5, 1, 0]).await;
    let mut greeting = [0; 2];
    timeout(LIMIT, socket.read_exact(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(greeting, [5, 0]);
    write_bounded(&mut socket, &[5, 1, 0, 1, 127, 0, 0, 1, 8]).await;
    assert!(
        timeout(Duration::from_secs(31), socket.read(&mut [0]))
            .await
            .is_err(),
        "incomplete CONNECT produced a response within 31 seconds"
    );
    assert!(controls.try_recv().is_err());
    assert!(messages.lock().unwrap().is_empty());
    cancel.cancel();
    assert_eof(&mut socket).await;
    assert!(controls.try_recv().is_err());
    drop(socket);
    drop(controls);
    task.finish().await;
    assert!(
        TcpListener::bind(addr).await.is_ok(),
        "cancelled listener address was not released"
    );
    peer.cleanup().await;
}
