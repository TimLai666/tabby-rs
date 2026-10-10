// Regression slot: failed resize must not discard queued transport output.
// The tail arrives over the real loopback transport right before the server
// disconnects; it is never fabricated into `pending`.
use crate::ssh::{input, model::SshResizeRequest, SshError};

use super::*;

#[tokio::test]
async fn live_resize_returns_ok_and_keeps_shell_loop_usable() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut f = Fixture::new().await;
        let (mut task, handle_input) = input::start(Arc::clone(&f._writer));
        let (resize_tx, resize_rx) = oneshot::channel();
        f.sender
            .send(SshControl::Resize(
                SshResizeRequest {
                    id: "fixture".into(),
                    columns: 132,
                    rows: 40,
                    pixel_width: None,
                    pixel_height: None,
                },
                resize_tx,
            ))
            .await
            .unwrap();
        let control = match f.next().await {
            Some(ShellEvent::Control(control)) => control,
            _ => panic!("live resize control must be selected before any output"),
        };
        let mut sftp = None;
        let routes = Arc::new(std::sync::Mutex::new(Default::default()));
        let binary: &[u8] = &[0x00, 0xff, 0x80, b'r'];
        let result = run_control(
            &mut f.reader,
            &mut f.pending,
            &mut f.channel_closed,
            &mut task,
            crate::ssh::handle_control(
                &mut f.handle,
                &f._writer,
                &mut sftp,
                "fixture",
                &routes,
                control,
            ),
            |_| true,
        )
        .await;
        let write_ok = tokio::time::timeout(Duration::from_secs(5), async {
            let (tx, rx) = oneshot::channel();
            handle_input.send((binary.to_vec(), tx)).await.unwrap();
            rx.await
        })
        .await;
        let mut echo = None;
        if let Some(ShellEvent::Channel(ChannelMsg::Data { data })) = f.next().await {
            echo = Some(data.to_vec());
        }
        let reply = resize_rx.await;
        drop(task);
        f.finish().await;
        assert!(result, "a live resize must keep the shell loop running");
        assert!(
            matches!(reply, Ok(Ok(()))),
            "live resize must report success: {reply:?}"
        );
        assert!(
            matches!(write_ok, Ok(Ok(Ok(())))),
            "input must stay usable after the resize: {write_ok:?}"
        );
        assert_eq!(
            echo.as_deref(),
            Some(binary),
            "the shell must still echo the exact input bytes"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn closed_transport_resize_delivers_error_but_keeps_loop_until_tail_drained() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut f = Fixture::new().await;
        let (mut task, _handle_input) = input::start(Arc::clone(&f._writer));
        let (resize_tx, resize_rx) = oneshot::channel();
        f.sender
            .send(SshControl::Resize(
                SshResizeRequest {
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
        // No pending output: the real loopback read side is still empty, so the
        // actual Resize control is selected by next_shell_event.
        let control = match f.next().await {
            Some(ShellEvent::Control(control)) => control,
            _ => panic!("resize control must be selected before the transport races"),
        };
        let tail: &[u8] = &[0x00, 0xff, 0x80];
        f.server
            .data(f.id, CryptoVec::from_slice(tail))
            .await
            .unwrap();
        f.server
            .disconnect(
                russh::Disconnect::ByApplication,
                "fixture race over".into(),
                "en".into(),
            )
            .await
            .unwrap();
        // Bounded yield-based wait for the real transport end, without driving
        // the channel reader or awaiting the handle itself.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !f.handle.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the real transport must close after the server disconnect");
        let mut sftp = None;
        let routes = Arc::new(std::sync::Mutex::new(Default::default()));
        let mut emitted = Vec::new();
        let result = run_control(
            &mut f.reader,
            &mut f.pending,
            &mut f.channel_closed,
            &mut task,
            crate::ssh::handle_control(
                &mut f.handle,
                &f._writer,
                &mut sftp,
                "fixture",
                &routes,
                control,
            ),
            |message| {
                if let ChannelMsg::Data { data } = message {
                    emitted.extend_from_slice(&data);
                }
                true
            },
        )
        .await;
        let mut drained = Vec::new();
        loop {
            match f.next().await {
                Some(ShellEvent::Channel(ChannelMsg::Data { data })) => {
                    drained.extend_from_slice(&data)
                }
                Some(ShellEvent::Channel(_)) => {}
                Some(ShellEvent::ChannelClosed) => {}
                Some(ShellEvent::Control(_)) => panic!("no late controls expected"),
                None => break,
            }
        }
        let reply = resize_rx.await;
        drop(task);
        f.finish().await;
        assert!(
            result,
            "a failed resize must retain the outer loop so queued output is drained"
        );
        assert!(
            matches!(reply, Ok(Err(SshError::Closed))),
            "the resize caller must receive Closed for the failed transport request: {reply:?}"
        );
        assert!(
            emitted.is_empty(),
            "run_control must not consume the network tail"
        );
        assert_eq!(
            drained, tail,
            "the outer loop must drain the exact tail before transport end"
        );
    })
    .await
    .unwrap();
}
