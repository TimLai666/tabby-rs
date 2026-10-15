//! Per-window caption snapshot delivery, independent of the native window handles.
use super::WindowsCaptionSnapshot;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::mpsc;

pub(super) type Sender = mpsc::UnboundedSender<WindowsCaptionSnapshot>;
pub(super) type Receiver = mpsc::UnboundedReceiver<WindowsCaptionSnapshot>;

pub(super) fn channel() -> (Sender, Receiver) {
    // Press/release transitions drive animation; none may be coalesced.
    // Publishing must not wait on the native window thread.
    mpsc::unbounded_channel()
}

pub(super) fn publish(sender: &Sender, snapshot: WindowsCaptionSnapshot) -> bool {
    sender.send(snapshot).is_ok()
}

pub(super) async fn deliver(
    mut receiver: Receiver,
    active: Arc<AtomicBool>,
    mut emit: impl FnMut(WindowsCaptionSnapshot) + Send + 'static,
) {
    while active.load(Ordering::Acquire) {
        let Some(snapshot) = receiver.recv().await else {
            break;
        };
        if !active.load(Ordering::Acquire) {
            break;
        }
        emit(snapshot);
    }
}
