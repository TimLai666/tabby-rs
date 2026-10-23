use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use super::Initializations;

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn pending<F: Future>(future: Pin<&mut F>) {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    assert!(matches!(future.poll(&mut context), Poll::Pending));
}

#[tokio::test]
async fn custom_windows_without_initialization_are_not_delayed() {
    assert!(Initializations::default().wait("tools").await.is_ok());
}

#[tokio::test]
async fn bootstrap_waits_for_its_own_native_initialization() {
    let registry = Initializations::default();
    let first = registry.begin("main").unwrap();
    let other = registry.begin("window-1").unwrap();
    let mut first_wait = Box::pin(registry.wait("main"));
    let mut other_wait = Box::pin(registry.wait("window-1"));
    pending(first_wait.as_mut());
    pending(other_wait.as_mut());
    first.complete(Ok(()));
    assert!(first_wait.await.is_ok());
    pending(other_wait.as_mut());
    other.complete(Ok(()));
    assert!(other_wait.await.is_ok());
}

#[tokio::test]
async fn initialization_errors_are_retained_and_cannot_be_overwritten() {
    let registry = Initializations::default();
    let ticket = registry.begin("main").unwrap();
    ticket.complete(Err("native initialization failed".into()));
    ticket.complete(Ok(()));
    assert_eq!(
        registry.wait("main").await.unwrap_err(),
        "native initialization failed"
    );
}

#[tokio::test]
async fn duplicate_registration_does_not_reset_an_existing_window() {
    let registry = Initializations::default();
    let ticket = registry.begin("main").unwrap();
    ticket.complete(Ok(()));
    assert!(registry.begin("main").is_err());
    assert!(registry.wait("main").await.is_ok());
}

#[tokio::test]
async fn destruction_cancels_waiters_and_late_completion_cannot_reach_a_reused_label() {
    let registry = Initializations::default();
    let old = registry.begin("main").unwrap();
    let mut old_wait = Box::pin(registry.wait("main"));
    pending(old_wait.as_mut());
    registry.remove("main");
    assert!(old_wait.await.is_err());
    let new = registry.begin("main").unwrap();
    let mut new_wait = Box::pin(registry.wait("main"));
    pending(new_wait.as_mut());
    old.complete(Ok(()));
    pending(new_wait.as_mut());
    new.complete(Ok(()));
    assert!(new_wait.await.is_ok());
}

#[tokio::test]
async fn pending_bootstrap_can_execute_on_a_runtime_task() {
    let registry = Arc::new(Initializations::default());
    let pending = registry.begin("main").unwrap();
    let for_task = registry.clone();
    let task = tokio::spawn(async move { for_task.wait("main").await });
    pending.complete(Ok(()));
    assert!(task.await.unwrap().is_ok());
}

#[tokio::test]
async fn first_finished_page_claims_one_ticket_before_initialization_completes() {
    let registry = Initializations::default();
    let ticket = registry.begin("main").unwrap();
    let claimed = registry
        .first_page_load("main")
        .expect("registered first page");
    assert!(Arc::ptr_eq(&ticket, &claimed));
    assert!(registry.first_page_load("main").is_none());
    assert!(!registry.can_present("main"));
    let mut waiting = Box::pin(claimed.wait());
    pending(waiting.as_mut());
    ticket.complete(Ok(()));
    assert!(waiting.await.is_ok());
    assert!(
        registry.first_page_load("main").is_none(),
        "reload must not replay startup presentation"
    );
}

#[test]
fn unknown_pages_and_duplicate_registration_do_not_consume_another_window() {
    let registry = Initializations::default();
    assert!(registry.first_page_load("main").is_none());
    registry.begin("main").unwrap();
    registry.begin("peer").unwrap();
    assert!(registry.first_page_load("main").is_some());
    assert!(registry.begin("main").is_err());
    assert!(registry.first_page_load("main").is_none());
    assert!(registry.first_page_load("peer").is_some());
}

#[tokio::test]
async fn removed_page_ticket_cannot_resolve_a_replacement_or_erase_its_first_load() {
    let registry = Initializations::default();
    let old = registry.begin("main").unwrap();
    let claimed = registry.first_page_load("main").unwrap();
    let mut waiting = Box::pin(claimed.wait());
    pending(waiting.as_mut());
    registry.remove("main");
    assert!(waiting.await.is_err());
    assert!(registry.first_page_load("main").is_none());
    let replacement = registry.begin("main").unwrap();
    let replacement_page = registry.first_page_load("main").unwrap();
    assert!(!Arc::ptr_eq(&claimed, &replacement_page));
    let mut replacement_wait = Box::pin(replacement_page.wait());
    old.complete(Ok(()));
    pending(replacement_wait.as_mut());
    replacement.complete(Err("owned initialization failure".into()));
    assert_eq!(
        replacement_wait.await.unwrap_err(),
        "owned initialization failure"
    );
    assert!(registry.first_page_load("main").is_none());
}
