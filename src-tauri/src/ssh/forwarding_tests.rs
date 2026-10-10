use std::{sync::Arc, time::Duration};

use russh::{client, server, Channel, ChannelId, ChannelMsg, CryptoVec};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{broadcast, oneshot},
};

fn loopback_key_pair() -> (russh::keys::PrivateKey, russh::keys::PublicKey) {
    let key =
        russh::keys::PrivateKey::random(&mut rand::rngs::OsRng, russh::keys::Algorithm::Ed25519)
            .unwrap();
    let public_key = key.public_key().clone();
    (key, public_key)
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

struct RejectingRemoteForwardServer;

impl server::Handler for RejectingRemoteForwardServer {
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

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
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

#[tokio::test]
async fn occupied_local_address_preserves_real_bind_failure_details() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let holder = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = holder.local_addr().unwrap();
        let host = address.ip().to_string();
        // The exact real bind failure for the same endpoint, converted to an AppError payload.
        let expected = TcpListener::bind((host.as_str(), address.port()))
            .await
            .expect_err("binding an occupied 127.0.0.1 port must fail")
            .to_string();
        let error = super::forwarding::bind_listener(&host, address.port())
            .await
            .expect_err("production bind_listener must fail on the occupied port");
        let value = serde_json::to_value(crate::error::AppError::from(error)).unwrap();
        assert_eq!(value["code"], "io");
        assert_eq!(
            value["details"].as_str().unwrap(),
            expected,
            "the bridge must preserve the real bind error instead of the generic transport message"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ephemeral_local_forward_supports_binary_roundtrip_and_rebind_after_drop() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = super::forwarding::bind_listener("127.0.0.1", 0)
            .await
            .expect("production bind_listener must start on an ephemeral port");
        let address = listener.local_addr().unwrap();
        assert_ne!(address.port(), 0);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut payload = [0u8; 5];
            socket.read_exact(&mut payload).await.unwrap();
            socket.write_all(&payload).await.unwrap();
            // Wait for the client to close first so the listening port is not left in TIME_WAIT.
            let mut closing = [0u8; 1];
            assert_eq!(socket.read(&mut closing).await.unwrap(), 0);
            // The listener is dropped here, releasing the port.
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        let payload = [0xde, 0xad, 0x00, 0xff, 0xbe];
        client.write_all(&payload).await.unwrap();
        let mut echoed = [0u8; 5];
        client.read_exact(&mut echoed).await.unwrap();
        assert_eq!(
            echoed, payload,
            "binary payload must round-trip through the listener"
        );
        client.shutdown().await.unwrap();
        server.await.unwrap();
        drop(client);
        let rebound = super::forwarding::bind_listener("127.0.0.1", address.port())
            .await
            .expect("the same address must be bindable once the listener is dropped");
        assert_eq!(
            rebound.local_addr().unwrap().port(),
            address.port(),
            "dropping the listener must release the same port"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rejected_remote_forward_preserves_rejection_details_and_session() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (host_key, public_key) = loopback_key_pair();
        let mut server_config = server::Config::default();
        server_config.keys.push(host_key);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let session =
                server::run_stream(Arc::new(server_config), tcp, RejectingRemoteForwardServer)
                    .await
                    .unwrap();
            let _ = session.await;
        });
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            address,
            PinnedClient(public_key),
        )
        .await
        .unwrap();
        assert!(handle.authenticate_none("fixture").await.unwrap().success());
        let channel = handle.channel_open_session().await.unwrap();
        let (mut reader, writer) = channel.split();

        // The locked russh Display of the same real rejection is the expected reason.
        let rejection = handle
            .tcpip_forward("127.0.0.1", 22000)
            .await
            .expect_err("fixture server must reject tcpip-forward")
            .to_string();
        assert_eq!(
            rejection,
            "The request was rejected by the other party",
            "the locked russh rejection reason must not be fabricated in this fixture"
        );

        let routes = Arc::new(std::sync::Mutex::new(Default::default()));
        let (cancel, _) = broadcast::channel(4);
        let (sender, receiver) = oneshot::channel();
        let keep_running = super::handle_control(
            &mut handle,
            &writer,
            &mut None,
            "fixture-connection",
            &routes,
            super::SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: 22100,
                target_address: "127.0.0.1".into(),
                target_port: 1,
                cancel,
                sender,
            },
        )
        .await;
        assert!(
            keep_running,
            "a rejected remote forward must not tear down the SSH session"
        );
        let result = receiver.await.unwrap();
        let error = result.expect_err("the rejected tcpip-forward must surface as an SshError");
        let value = serde_json::to_value(crate::error::AppError::from(error)).unwrap();
        assert_eq!(value["code"], "io");
        assert_eq!(
            value["details"].as_str().unwrap(),
            rejection,
            "the bridge must preserve the russh rejection reason instead of the shell-channel message"
        );
        assert!(
            routes.lock().unwrap().is_empty(),
            "a rejected remote forward must not register a route"
        );

        let payload = [0xde, 0xad, 0x00, 0xff, 0xbe];
        let mut sink = writer.make_writer();
        sink.write_all(&payload).await.unwrap();
        sink.flush().await.unwrap();
        let mut echoed = Vec::new();
        while echoed.len() < payload.len() {
            match reader.wait().await {
                Some(ChannelMsg::Data { data }) => echoed.extend_from_slice(&data),
                Some(other) => panic!("unexpected channel message during round-trip: {other:?}"),
                None => panic!("the SSH channel ended before the echo"),
            }
        }
        assert_eq!(&echoed[..], &payload[..]);

        handle
            .disconnect(russh::Disconnect::ByApplication, "fixture done", "en")
            .await
            .unwrap();
        tasks.join_next().await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
