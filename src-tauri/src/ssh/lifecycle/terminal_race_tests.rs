// Deterministically exercise a reply becoming ready between select branch polls.
// The real loopback reader is used; this is not a global-request wire race probe.
use std::future::{poll_fn, Future};

use super::*;
use crate::ssh::input;

fn ready_after_first_pending(
    expected: bool,
    reply: oneshot::Sender<bool>,
) -> impl Future<Output = bool> {
    let (release, receive) = oneshot::channel();
    let mut release = Some(release);
    let mut operation = Box::pin(async move {
        let result = receive.await.unwrap();
        reply.send(result).unwrap();
        result
    });
    poll_fn(move |cx| {
        let polled = operation.as_mut().poll(cx);
        if polled.is_pending() {
            if let Some(release) = release.take() {
                // A conforming wake makes the result available after the first poll.
                release.send(expected).unwrap();
            }
        }
        polled
    })
}

#[tokio::test]
async fn reply_ready_between_operation_poll_and_eof_is_delivered_before_end() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for expected in [true, false] {
            let mut f = Fixture::new().await;
            let (mut task, _input) = input::start(Arc::clone(&f._writer));
            f.pending.push_back(ChannelMsg::Eof);
            let (reply, mut received) = oneshot::channel();
            let mut emitted = 0;
            let result = run_control(
                &mut f.reader,
                &mut f.pending,
                &mut f.channel_closed,
                &mut task,
                ready_after_first_pending(expected, reply),
                |_| {
                    emitted += 1;
                    true
                },
            )
            .await;
            let delivered = received.try_recv();
            let queued = f.pending.len();
            if result {
                assert!(
                    f.next().await.is_none(),
                    "observed EOF must still end the shell"
                );
            }
            drop(task);
            f.finish().await;
            assert!(
                matches!(delivered, Ok(actual) if actual == expected),
                "reply became ready before EOF was selected but was lost: {delivered:?}"
            );
            assert_eq!(result, expected, "preserve the available operation result");
            assert_eq!(emitted, 0);
            assert_eq!(
                queued, 1,
                "retain EOF for the outer loop when delivering the result"
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reply_ready_before_emit_refusal_survives_while_output_failure_ends_loop() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for expected in [true, false] {
            let mut f = Fixture::new().await;
            let (mut task, input) = input::start(Arc::clone(&f._writer));
            f.pending.push_back(ChannelMsg::Data {
                data: CryptoVec::from_slice(&[0x00, 0xff, 0x80]),
            });
            let (reply, mut received) = oneshot::channel();
            let mut emitted = 0;
            let result = run_control(
                &mut f.reader,
                &mut f.pending,
                &mut f.channel_closed,
                &mut task,
                ready_after_first_pending(expected, reply),
                |message| {
                    assert!(matches!(message, ChannelMsg::Data { .. }));
                    emitted += 1;
                    false
                },
            )
            .await;
            let delivered = received.try_recv();
            let input_stopped = input.is_closed();
            drop(task);
            f.finish().await;
            assert!(
                matches!(delivered, Ok(actual) if actual == expected),
                "available reply must survive an actual output failure: {delivered:?}"
            );
            assert!(!result, "once output emission fails, the shell must end");
            assert_eq!(emitted, 1, "do not retry or continue failed output");
            assert!(
                input_stopped,
                "output failure must stop input even with a ready reply"
            );
        }
    })
    .await
    .unwrap();
}

fn spend_cooperative_budget(cx: &mut std::task::Context<'_>) {
    // Use public guards, with a cap so unconstrained test execution fails promptly.
    for _ in 0..4096 {
        match tokio::task::coop::poll_proceed(cx) {
            std::task::Poll::Ready(guard) => guard.made_progress(),
            std::task::Poll::Pending => break,
        }
    }
    assert!(
        !tokio::task::coop::has_budget_remaining(),
        "budget test requires a bounded cooperative task budget"
    );
}

#[tokio::test]
#[should_panic(expected = "budget test requires a bounded cooperative task budget")]
async fn budget_fixture_rejects_unconstrained_execution() {
    tokio::task::unconstrained(poll_fn(|cx| {
        spend_cooperative_budget(cx);
        std::task::Poll::Ready(())
    }))
    .await;
}

// A ready Tokio oneshot can yield when this task has spent its cooperative budget.
// Exercise that scheduler state, not the timing of a remote global-request reply.
#[tokio::test]
async fn terminal_completion_preserves_ready_reply_after_budget_is_spent() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for eof in [true, false] {
            for expected in [true, false] {
                let mut f = Fixture::new().await;
                let (mut task, input) = input::start(Arc::clone(&f._writer));
                // Match a control path whose input was already stopped by its caller.
                task.stop().await;
                f.pending.push_back(if eof {
                    ChannelMsg::Eof
                } else {
                    ChannelMsg::Data { data: CryptoVec::from_slice(b"refused") }
                });
                let (reply, mut received) = oneshot::channel();
                let mut inner = Box::pin(ready_after_first_pending(expected, reply));
                let mut first_pending = true;
                let operation = poll_fn(move |cx| {
                    let result = inner.as_mut().poll(cx);
                    if first_pending && result.is_pending() {
                        first_pending = false;
                        spend_cooperative_budget(cx);
                    }
                    result
                });
                let mut emitted = 0;
                let result = run_control(
                    &mut f.reader,
                    &mut f.pending,
                    &mut f.channel_closed,
                    &mut task,
                    operation,
                    |_| { emitted += 1; false },
                ).await;
                let delivered = received.try_recv();
                let queued = f.pending.len();
                let stopped = input.is_closed();
                if eof && result {
                    assert!(f.next().await.is_none());
                }
                drop(task);
                f.finish().await;
                assert!(matches!(delivered, Ok(actual) if actual == expected),
                    "a ready reply was lost solely because the task budget was spent (EOF={eof}, result={expected}): {delivered:?}");
                assert_eq!(result, eof && expected);
                assert_eq!(emitted, usize::from(!eof));
                assert_eq!(queued, usize::from(eof));
                assert!(stopped);
            }
        }
    }).await.unwrap();
}
