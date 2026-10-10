// Real SSH lifecycle regression cases for pending control operations.
use std::{
    future::Future,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::*;
use crate::ssh::{
    input::{self, InputTask},
    SshError,
};

const STEP: Duration = Duration::from_secs(3);
const CASE: Duration = Duration::from_secs(20);

fn pending_operation() -> (
    impl Future<Output = bool>,
    oneshot::Sender<bool>,
    Arc<AtomicBool>,
) {
    let (release, wait) = oneshot::channel::<bool>();
    let completed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&completed);
    let operation = async move {
        let result = wait.await.unwrap_or(false);
        flag.store(true, Ordering::SeqCst);
        result
    };
    (operation, release, completed)
}

fn collector() -> (
    impl FnMut(ChannelMsg) -> bool,
    mpsc::UnboundedReceiver<ChannelMsg>,
) {
    let (sender, receiver) = mpsc::unbounded_channel();
    (move |message| sender.send(message).is_ok(), receiver)
}

async fn emitted(receiver: &mut mpsc::UnboundedReceiver<ChannelMsg>) -> ChannelMsg {
    tokio::time::timeout(STEP, receiver.recv())
        .await
        .expect("output must reach emit while the operation is still pending")
        .expect("emit channel closed")
}

fn assert_data(message: ChannelMsg, expected: &[u8]) {
    match message {
        ChannelMsg::Data { data } => assert_eq!(&data[..], expected),
        _ => panic!("expected Data"),
    }
}

fn assert_extended(message: ChannelMsg, expected_ext: u32, expected: &[u8]) {
    match message {
        ChannelMsg::ExtendedData { ext, data } => {
            assert_eq!(ext, expected_ext);
            assert_eq!(&data[..], expected);
        }
        _ => panic!("expected ExtendedData"),
    }
}

struct Stalled {
    task: InputTask,
    input: mpsc::Sender<input::ShellWrite>,
    write_reply: oneshot::Receiver<Result<(), SshError>>,
    queued: Vec<oneshot::Receiver<Result<(), SshError>>>,
}

// With a one-byte window the echo of the first byte proves the 8192-byte write is blocked.
async fn stall_input(f: &mut Fixture) -> Stalled {
    let (task, input) = input::start(Arc::clone(&f._writer));
    let (tx, mut write_reply) = oneshot::channel();
    input.send((vec![b'x'; 8192], tx)).await.unwrap();
    f.expect_data(b"x").await;
    assert!(matches!(
        write_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
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
    Stalled {
        task,
        input,
        write_reply,
        queued,
    }
}

#[tokio::test]
async fn pending_operation_emits_data_and_extended_data_in_order() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::new().await;
        let id = f.id;
        let (mut input_task, _input) = input::start(Arc::clone(&f._writer));
        let (operation, release, completed) = pending_operation();
        let (emit, mut rx) = collector();
        let Fixture {
            reader,
            pending,
            channel_closed,
            server,
            ..
        } = &mut f;
        let binary_a: &[u8] = &[0x00, 0xff, 0x80, 0xc3, 0x28, b'a'];
        let binary_b: &[u8] = &[0xfe, 0x00, 0x01, 0x7f, 0xa0];
        let binary_c: &[u8] = &[0xed, 0xa0, 0x80, 0x00, 0x00];
        let driver = async {
            server
                .data(id, CryptoVec::from_slice(binary_a))
                .await
                .unwrap();
            server
                .extended_data(id, 1, CryptoVec::from_slice(binary_b))
                .await
                .unwrap();
            server
                .data(id, CryptoVec::from_slice(binary_c))
                .await
                .unwrap();
            assert_data(emitted(&mut rx).await, binary_a);
            assert_extended(emitted(&mut rx).await, 1, binary_b);
            assert_data(emitted(&mut rx).await, binary_c);
            assert!(!completed.load(Ordering::SeqCst));
            release.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(
            run_control(
                reader,
                pending,
                channel_closed,
                &mut input_task,
                operation,
                emit
            ),
            driver
        );
        assert!(result);
        assert!(rx.try_recv().is_err(), "no unexpected extra output");
        assert!(!f.channel_closed);
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn eof_during_pending_operation_drains_output_and_ends_without_completion() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::with_window(1).await;
        let id = f.id;
        let stalled = stall_input(&mut f).await;
        let Stalled {
            mut task,
            input,
            write_reply,
            queued,
        } = stalled;
        let (tx, _rx) = oneshot::channel();
        let mut blocked_send = Box::pin(input.send((vec![b'z'], tx)));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut blocked_send)
                .await
                .is_err()
        );
        f.pending.push_back(ChannelMsg::Data {
            data: CryptoVec::from_slice(b"buffered"),
        });
        f.server
            .data(id, CryptoVec::from_slice(b"remote"))
            .await
            .unwrap();
        f.server.eof(id).await.unwrap();
        let (operation, release, completed) = pending_operation();
        let (emit, mut rx) = collector();
        let result = tokio::time::timeout(
            STEP,
            run_control(
                &mut f.reader,
                &mut f.pending,
                &mut f.channel_closed,
                &mut task,
                operation,
                emit,
            ),
        )
        .await
        .expect("EOF must end the helper promptly while the operation is pending");
        assert!(!result, "EOF must end the shell");
        assert_data(rx.try_recv().expect("buffered output first"), b"buffered");
        assert_data(rx.try_recv().expect("remote output second"), b"remote");
        assert!(rx.try_recv().is_err());
        assert!(
            !completed.load(Ordering::SeqCst),
            "operation must not complete"
        );
        assert!(release.is_closed(), "operation must be dropped");
        assert!(input.is_closed(), "input must be stopped");
        assert!(write_reply.await.is_err(), "blocked input waiter released");
        for reply in queued {
            assert!(reply.await.is_err());
        }
        assert!(blocked_send.await.is_err(), "full-queue sender released");
        drop(task);
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn remote_close_during_pending_operation_stops_input_and_retains_operation() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::with_window(1).await;
        let id = f.id;
        let Stalled {
            mut task,
            input,
            write_reply,
            queued,
        } = stall_input(&mut f).await;
        let (tx, _rx) = oneshot::channel();
        let mut blocked_send = Box::pin(input.send((vec![b'z'], tx)));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut blocked_send)
                .await
                .is_err()
        );
        let (operation, release, completed) = pending_operation();
        let (emit, _rx) = collector();
        let Fixture {
            reader,
            pending,
            channel_closed,
            server,
            closed,
            ..
        } = &mut f;
        let driver = async {
            server.close(id).await.unwrap();
            tokio::time::timeout(STEP, input.closed())
                .await
                .expect("remote CLOSE must stop input");
            let ack = tokio::time::timeout(STEP, closed.recv())
                .await
                .expect("peer must receive the CLOSE acknowledgement");
            assert_eq!(ack, Some(id));
            assert!(!release.is_closed(), "operation must remain pending");
            assert!(!completed.load(Ordering::SeqCst));
            release.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(
            run_control(reader, pending, channel_closed, &mut task, operation, emit),
            driver
        );
        assert!(result, "operation result must be preserved after CLOSE");
        assert!(completed.load(Ordering::SeqCst));
        assert!(f.channel_closed);
        assert!(write_reply.await.is_err(), "blocked input waiter released");
        for reply in queued {
            assert!(reply.await.is_err());
        }
        assert!(blocked_send.await.is_err(), "full-queue sender released");
        let another = tokio::time::timeout(STEP, f.handle.channel_open_session())
            .await
            .expect("transport must remain usable")
            .unwrap();
        another.close().await.unwrap();
        drop(task);
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn exit_status_and_signal_keep_pending_operation_and_input_alive() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::new().await;
        let id = f.id;
        let (mut task, input) = input::start(Arc::clone(&f._writer));
        let (operation, release, completed) = pending_operation();
        let (emit, mut rx) = collector();
        let Fixture {
            reader,
            pending,
            channel_closed,
            server,
            ..
        } = &mut f;
        let driver = async {
            server.exit_status_request(id, 42).await.unwrap();
            server
                .exit_signal_request(id, russh::Sig::TERM, false, "fixture".into(), "en".into())
                .await
                .unwrap();
            server
                .data(id, CryptoVec::from_slice(b"after status and signal"))
                .await
                .unwrap();
            assert_data(emitted(&mut rx).await, b"after status and signal");
            assert!(!completed.load(Ordering::SeqCst));
            assert!(!input.is_closed(), "status/signal must not stop input");
            let (tx, reply) = oneshot::channel();
            input.send((b"ping".to_vec(), tx)).await.unwrap();
            assert!(tokio::time::timeout(STEP, reply)
                .await
                .unwrap()
                .unwrap()
                .is_ok());
            assert_data(emitted(&mut rx).await, b"ping");
            release.send(true).unwrap();
        };
        let (result, ()) = tokio::join!(
            run_control(reader, pending, channel_closed, &mut task, operation, emit),
            driver
        );
        assert!(result);
        assert!(!f.channel_closed);
        assert!(!input.is_closed());
        drop(task);
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn completed_operation_results_are_preserved_without_output() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::new().await;
        let (mut task, _input) = input::start(Arc::clone(&f._writer));
        for yielding in [false, true] {
            for expected in [true, false] {
                let (emit, mut rx) = collector();
                let result = tokio::time::timeout(
                    STEP,
                    run_control(
                        &mut f.reader,
                        &mut f.pending,
                        &mut f.channel_closed,
                        &mut task,
                        async move {
                            if yielding {
                                tokio::task::yield_now().await;
                            }
                            expected
                        },
                        emit,
                    ),
                )
                .await
                .expect("completed operation must not hang without output");
                assert_eq!(result, expected);
                assert!(rx.try_recv().is_err());
                assert!(!f.channel_closed);
            }
        }
        drop(task);
        f.finish().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn emit_refusal_ends_pending_operation_and_stops_input() {
    tokio::time::timeout(CASE, async {
        let mut f = Fixture::with_window(1).await;
        let id = f.id;
        let Stalled {
            mut task,
            input,
            write_reply,
            queued,
        } = stall_input(&mut f).await;
        let (tx, _rx) = oneshot::channel();
        let mut blocked_send = Box::pin(input.send((vec![b'z'], tx)));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut blocked_send)
                .await
                .is_err()
        );
        f.server
            .data(id, CryptoVec::from_slice(b"first"))
            .await
            .unwrap();
        f.server
            .data(id, CryptoVec::from_slice(b"second"))
            .await
            .unwrap();
        let (operation, release, completed) = pending_operation();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let result = tokio::time::timeout(
            STEP,
            run_control(
                &mut f.reader,
                &mut f.pending,
                &mut f.channel_closed,
                &mut task,
                operation,
                move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    false
                },
            ),
        )
        .await
        .expect("a refused emit must end the helper promptly");
        assert!(!result);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no emit after refusal");
        assert!(!completed.load(Ordering::SeqCst));
        assert!(release.is_closed(), "operation must be dropped");
        assert!(input.is_closed(), "input must be stopped");
        assert!(write_reply.await.is_err(), "blocked input waiter released");
        for reply in queued {
            assert!(reply.await.is_err());
        }
        assert!(blocked_send.await.is_err(), "full-queue sender released");
        drop(task);
        f.finish().await;
    })
    .await
    .unwrap();
}
