use super::super::{
    CaptionButton, CaptionButtonLayout, CaptionLayout, CaptionPointerState, CaptionRect,
    WindowsCaptionSnapshot,
};
use super::{channel, deliver, publish};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime")
}

fn pointer(revision: u64, pointer: CaptionPointerState) -> WindowsCaptionSnapshot {
    WindowsCaptionSnapshot {
        revision,
        visible: true,
        scale: 1.25,
        layout: CaptionLayout {
            clip: CaptionRect {
                x: 12.5,
                y: 1.25,
                width: 56.25,
                height: 38.75,
            },
            buttons: vec![CaptionButtonLayout {
                button: CaptionButton::Minimize,
                bounds: CaptionRect {
                    x: 12.5,
                    y: 1.25,
                    width: 56.25,
                    height: 38.75,
                },
                enabled: true,
            }],
        },
        pointer,
        maximized: revision == 3,
        active: revision != 2,
    }
}

fn hovered() -> WindowsCaptionSnapshot {
    pointer(
        1,
        CaptionPointerState {
            hovered: Some(CaptionButton::Minimize),
            pressed: None,
        },
    )
}

fn pressed() -> WindowsCaptionSnapshot {
    pointer(
        2,
        CaptionPointerState {
            hovered: Some(CaptionButton::Minimize),
            pressed: Some(CaptionButton::Minimize),
        },
    )
}

fn released() -> WindowsCaptionSnapshot {
    pointer(
        3,
        CaptionPointerState {
            hovered: Some(CaptionButton::Minimize),
            pressed: None,
        },
    )
}

type Log = Arc<Mutex<Vec<WindowsCaptionSnapshot>>>;

fn sink() -> (Log, impl FnMut(WindowsCaptionSnapshot) + Send + 'static) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&log);
    (log, move |snapshot| writer.lock().unwrap().push(snapshot))
}

fn recorded(log: &Log) -> Vec<WindowsCaptionSnapshot> {
    log.lock().unwrap().clone()
}

#[test]
fn pre_poll_replays_every_revision_in_order() {
    runtime().block_on(async {
        let (sender, receiver) = channel();
        let hovered = hovered();
        let pressed = pressed();
        let released = released();
        assert!(publish(&sender, hovered.clone()));
        assert!(publish(&sender, pressed.clone()));
        assert!(publish(&sender, released.clone()));
        drop(sender);

        let (log, emit) = sink();
        timeout(
            Duration::from_secs(1),
            deliver(receiver, Arc::new(AtomicBool::new(true)), emit),
        )
        .await
        .expect("deliver returns once every sender has dropped");

        assert_eq!(
            recorded(&log),
            vec![hovered, pressed, released],
            "a receiver that starts polling late must still receive every distinct revision"
        );
    });
}

#[test]
fn waiting_relay_preserves_back_to_back_publishes() {
    runtime().block_on(async {
        let (sender, receiver) = channel();
        let hovered = hovered();
        let pressed = pressed();
        let released = released();
        assert!(publish(&sender, hovered.clone()));

        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready = Some(ready_tx);
        let handle = tokio::spawn(deliver(receiver, Arc::new(AtomicBool::new(true)), {
            let log = Arc::clone(&log);
            move |snapshot| {
                log.lock().unwrap().push(snapshot);
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
        }));

        timeout(Duration::from_secs(1), ready_rx)
            .await
            .expect("the relay emits the first snapshot")
            .expect("the relay signals its first emit");

        assert!(publish(&sender, pressed.clone()));
        assert!(publish(&sender, released.clone()));
        drop(sender);

        timeout(Duration::from_secs(1), handle)
            .await
            .expect("the relay ends after its sender drops")
            .expect("the relay task completes");

        assert_eq!(
            recorded(&log),
            vec![hovered, pressed, released],
            "a waiting relay must not collapse revisions published in one un-awaited run"
        );
    });
}

#[test]
fn empty_closed_channel_returns_without_fabricating_a_snapshot() {
    runtime().block_on(async {
        let (sender, receiver) = channel();
        drop(sender);
        let (log, emit) = sink();
        timeout(
            Duration::from_secs(1),
            deliver(receiver, Arc::new(AtomicBool::new(true)), emit),
        )
        .await
        .expect("an empty closed channel returns");
        assert!(recorded(&log).is_empty());
    });
}

#[test]
fn inactive_relay_returns_without_emitting() {
    runtime().block_on(async {
        let (sender, receiver) = channel();
        assert!(publish(&sender, hovered()));

        let (log, emit) = sink();
        timeout(
            Duration::from_secs(1),
            deliver(receiver, Arc::new(AtomicBool::new(false)), emit),
        )
        .await
        .expect("an inactive relay returns immediately");

        assert!(
            recorded(&log).is_empty(),
            "an inactive relay must not emit while its sender stays alive"
        );
        drop(sender);
    });
}

#[test]
fn callback_deactivation_ends_without_waiting_for_the_sender() {
    runtime().block_on(async {
        let (sender, receiver) = channel();
        let first = hovered();
        assert!(publish(&sender, first.clone()));
        assert!(publish(&sender, pressed()));
        assert!(publish(&sender, released()));

        let active = Arc::new(AtomicBool::new(true));
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready = Some(ready_tx);
        let handle = tokio::spawn(deliver(receiver, Arc::clone(&active), {
            let log = Arc::clone(&log);
            let active = Arc::clone(&active);
            move |snapshot| {
                log.lock().unwrap().push(snapshot);
                active.store(false, Ordering::Release);
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
            }
        }));

        timeout(Duration::from_secs(1), ready_rx)
            .await
            .expect("the relay emits the first snapshot")
            .expect("the relay signals its first emit");

        timeout(Duration::from_secs(1), handle)
            .await
            .expect("deactivation stops the relay without waiting for the sender")
            .expect("the relay task completes");

        assert_eq!(recorded(&log), vec![first]);
        drop(sender);
    });
}

#[test]
fn receiver_drop_reports_false_and_channels_stay_isolated() {
    runtime().block_on(async {
        let (sender_a, receiver_a) = channel();
        let (sender_b, receiver_b) = channel();
        let a = hovered();
        let b = pressed();
        assert!(publish(&sender_a, a));
        assert!(publish(&sender_b, b.clone()));

        drop(receiver_a);
        assert!(
            !publish(&sender_a, released()),
            "publish reports false once its receiver is gone"
        );

        let (log, emit) = sink();
        drop(sender_b);
        timeout(
            Duration::from_secs(1),
            deliver(receiver_b, Arc::new(AtomicBool::new(true)), emit),
        )
        .await
        .expect("the surviving window channel drains and returns");

        assert_eq!(
            recorded(&log),
            vec![b],
            "one window channel must never observe another window's snapshot"
        );
        drop(sender_a);
    });
}
