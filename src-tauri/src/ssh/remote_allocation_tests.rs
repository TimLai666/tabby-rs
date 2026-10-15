use super::*;

async fn start_allocated(
    peer: &mut Peer,
    target: SocketAddr,
    cancel: CancelSignal,
) -> Result<u16, super::super::model::SshError> {
    let (sender, receiver) = oneshot::channel();
    let keep = super::super::handle_control(
        &mut peer.handle,
        &peer.writer,
        &mut None,
        &peer.connection_id,
        &peer.routes,
        SshControl::StartRemoteForward {
            bind_host: "127.0.0.1".into(),
            bind_port: 0,
            target_address: target.ip().to_string(),
            target_port: target.port(),
            cancel,
            sender,
        },
    )
    .await;
    assert!(keep, "invalid allocation must keep the SSH session alive");
    receiver.await.expect("allocation result sender dropped")
}

fn assert_error_contains_raw_number(error: &str, value: u32) {
    let expected = value.to_string();
    assert!(
        error
            .split(|character: char| !character.is_ascii_digit())
            .any(|token| token == expected),
        "forwarding error {error:?} omitted raw allocation value {value}"
    );
}

fn assert_no_allocated_route(routes: &Routes, connection_id: &str) {
    let routes = routes.lock().unwrap();
    assert!(
        routes
            .keys()
            .all(|(owner, address, _)| owner != connection_id || address != "127.0.0.1"),
        "invalid allocation left a route for this connection"
    );
}

async fn assert_rejected_allocation_recovers(value: u32) {
    let routes = Routes::default();
    let mut peer = Peer::connect(routes.clone(), &format!("allocation-reject-{value}")).await;
    *peer.state.allocation_reply.lock().unwrap() = Some(value);
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let target_address = target.local_addr().unwrap();

    let result = start_allocated(&mut peer, target_address, CancelSignal::default()).await;
    let error = match result {
        Err(super::super::model::SshError::Forwarding(error)) => error,
        other => panic!("allocation {value} must fail with Forwarding, got {other:?}"),
    };
    assert_error_contains_raw_number(&error, value);
    assert_no_allocated_route(&routes, &peer.connection_id);
    assert!(
        peer.state.cancel_requests.lock().unwrap().is_empty(),
        "invalid allocation sent a remote cancellation"
    );

    peer.shell_echo(b"shell survives invalid allocation").await;
    let port = peer
        .start_remote(0, target_address, CancelSignal::default())
        .await;
    assert_ne!(port, 0, "normal allocation must recover after rejection");
    assert!(routes.lock().unwrap().contains_key(&(
        peer.connection_id.clone(),
        "127.0.0.1".into(),
        u32::from(port)
    )));
    echo_remote(port, &target, &[0x34, 0x00, 0xff, 0x91, 0x7a]).await;
    peer.shell_echo(b"shell survives normal allocation recovery")
        .await;

    peer.cleanup().await;
    drop(target);
}

#[tokio::test]
async fn allocated_remote_port_zero_is_rejected_and_session_recovers() {
    // SSH success with allocation value zero omits the port body; it does not
    // represent an explicit four-byte zero allocation field.
    assert_rejected_allocation_recovers(0).await;
}

#[tokio::test]
async fn allocated_remote_port_65536_is_rejected_and_session_recovers() {
    assert_rejected_allocation_recovers(65_536).await;
}

#[tokio::test]
async fn allocated_remote_port_65537_is_rejected_and_session_recovers() {
    assert_rejected_allocation_recovers(65_537).await;
}

#[tokio::test]
async fn allocated_remote_port_u32_max_is_rejected_and_session_recovers() {
    assert_rejected_allocation_recovers(u32::MAX).await;
}

async fn assert_forced_allocation_works(value: u16) {
    let routes = Routes::default();
    let mut peer = Peer::connect(routes.clone(), &format!("allocation-boundary-{value}")).await;
    *peer.state.allocation_reply.lock().unwrap() = Some(u32::from(value));
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let target_address = target.local_addr().unwrap();

    let result = start_allocated(&mut peer, target_address, CancelSignal::default()).await;
    assert_eq!(
        result.unwrap(),
        value,
        "allocated result must retain the wire port"
    );
    let key = (
        peer.connection_id.clone(),
        "127.0.0.1".to_owned(),
        u32::from(value),
    );
    let route = routes
        .lock()
        .unwrap()
        .get(&key)
        .cloned()
        .expect("successful allocation must install the exact route");
    assert_eq!(route.target_address, target_address.ip().to_string());
    assert_eq!(route.target_port, target_address.port());

    let channel = peer.open_forwarded("127.0.0.1", u32::from(value)).await;
    echo_on_channel(channel, &target, &[0x11, 0x00, 0xe4, 0xff, 0x82]).await;
    peer.shell_echo(b"shell survives boundary allocation").await;
    assert!(peer.state.cancel_requests.lock().unwrap().is_empty());

    peer.cleanup().await;
    drop(target);
}

#[tokio::test]
async fn allocated_remote_port_one_and_u16_max_are_usable() {
    assert_forced_allocation_works(1).await;
    assert_forced_allocation_works(u16::MAX).await;
}

#[tokio::test]
async fn rejected_u32_allocation_preserves_existing_same_owner_route() {
    let routes = Routes::default();
    let mut peer = Peer::connect(routes.clone(), "allocation-existing-route").await;
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let target_address = target.local_addr().unwrap();
    let route_key = (peer.connection_id.clone(), "127.0.0.1".to_owned(), 1);
    insert_route(
        &routes,
        &peer.connection_id,
        1,
        target_address,
        CancelSignal::default(),
        None,
    );
    let original_route = routes.lock().unwrap().get(&route_key).unwrap().clone();
    *peer.state.allocation_reply.lock().unwrap() = Some(65_537);

    let result = start_allocated(&mut peer, target_address, CancelSignal::default()).await;
    let error = match result {
        Err(super::super::model::SshError::Forwarding(error)) => error,
        other => panic!("allocation 65537 must fail with Forwarding, got {other:?}"),
    };
    assert_error_contains_raw_number(&error, 65_537);
    assert_eq!(routes.lock().unwrap().len(), 1);
    assert!(Arc::ptr_eq(
        routes.lock().unwrap().get(&route_key).unwrap(),
        &original_route
    ));
    assert!(peer.state.cancel_requests.lock().unwrap().is_empty());

    peer.shell_echo(b"shell survives route-preserving rejection")
        .await;
    let channel = peer.open_forwarded("127.0.0.1", 1).await;
    echo_on_channel(channel, &target, &[0x55, 0x00, 0xfe, 0xa1]).await;

    peer.cleanup().await;
    drop(target);
}

#[tokio::test]
async fn invalid_allocation_closes_an_early_channel_without_target_connection() {
    timeout(Duration::from_secs(15), async {
        let routes = Routes::default();
        let mut peer = Peer::connect(routes.clone(), "allocation-early-invalid").await;
        *peer.state.allocation_reply.lock().unwrap() = Some(65_537);
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_address = target.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        let cancel = CancelSignal::default();
        let wake = Arc::new(ReplyWake {
            ready: AtomicBool::new(false),
            notify: Notify::new(),
        });
        let mut sftp = None;
        let mut operation = Box::pin(super::super::handle_control(
            &mut peer.handle,
            &peer.writer,
            &mut sftp,
            &peer.connection_id,
            &routes,
            SshControl::StartRemoteForward {
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
                target_address: target_address.ip().to_string(),
                target_port: target_address.port(),
                cancel,
                sender,
            },
        ));

        poll_control_once(operation.as_mut(), wake.clone());
        timeout(LIMIT, async {
            loop {
                if peer.state.allocation_reply.lock().unwrap().is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("peer did not consume the forced allocation reply");

        let server_handle = peer.server_handle.clone();
        let mut early = timeout(
            LIMIT,
            server_handle.channel_open_forwarded_tcpip("127.0.0.1", 65_537, "127.0.0.1", 1234),
        )
        .await
        .unwrap()
        .unwrap();
        let early_id = early.id();
        assert_channel_waits_without_target(&mut peer.close_events, &mut early, &target).await;
        assert!(!peer
            .state
            .closed_channels
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(&early_id));

        wait_for_wake(&wake).await;
        assert!(timeout(LIMIT, operation.as_mut())
            .await
            .expect("allocation control did not resume"));
        drop(operation);
        let error = timeout(LIMIT, receiver)
            .await
            .expect("allocation result was not sent")
            .expect("allocation result sender dropped");
        let message = match error {
            Err(super::super::model::SshError::Forwarding(message)) => message,
            other => panic!("invalid allocation must return Forwarding, got {other:?}"),
        };
        assert_error_contains_raw_number(&message, 65_537);

        wait_for_channel_close(&mut peer.close_events, &peer.state, early_id).await;
        assert!(
            timeout(Duration::from_millis(250), target.accept())
                .await
                .is_err(),
            "invalid early channel connected to its target"
        );
        assert!(
            routes
                .lock()
                .unwrap()
                .keys()
                .all(|(owner, _, _)| { owner != &peer.connection_id }),
            "invalid allocation left an owned route"
        );
        assert!(peer.state.cancel_requests.lock().unwrap().is_empty());

        peer.shell_echo(&[0x71, 0x00, 0xff, 0x92]).await;
        let port = peer
            .start_remote(0, target_address, CancelSignal::default())
            .await;
        assert_ne!(port, 0, "normal remote allocation must recover");
        assert!(routes.lock().unwrap().contains_key(&(
            peer.connection_id.clone(),
            "127.0.0.1".into(),
            u32::from(port)
        )));
        echo_remote(port, &target, &[0xa3, 0x00, 0xff, 0x11, 0x80]).await;
        assert!(peer.state.cancel_requests.lock().unwrap().is_empty());

        peer.cleanup().await;
        drop(target);
    })
    .await
    .expect("invalid allocation early-channel scenario exceeded 15 seconds");
}
