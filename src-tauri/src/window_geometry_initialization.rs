use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tokio::sync::watch;

type Outcome = Option<Result<(), String>>;

#[derive(Default)]
pub(crate) struct Initializations {
    windows: Mutex<HashMap<String, Arc<PendingInitialization>>>,
}

pub(crate) struct PendingInitialization {
    outcome: watch::Sender<Outcome>,
    page_finished: AtomicBool,
}

impl Initializations {
    /// Claim only the first finished page for a registered window.
    pub(crate) fn first_page_load(&self, label: &str) -> Option<Arc<PendingInitialization>> {
        let windows = self
            .windows
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let ticket = windows.get(label)?;
        if ticket.page_finished.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(ticket.clone())
    }

    pub(crate) fn begin(&self, label: &str) -> Result<Arc<PendingInitialization>, String> {
        let mut windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if windows.contains_key(label) {
            return Err(format!("window geometry already registered for {label}"));
        }
        let (outcome, _) = watch::channel(None);
        let pending = Arc::new(PendingInitialization {
            outcome,
            page_finished: AtomicBool::new(false),
        });
        windows.insert(label.into(), pending.clone());
        Ok(pending)
    }

    pub(crate) fn can_present(&self, label: &str) -> bool {
        let windows = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        windows.get(label).map_or(true, |pending| {
            matches!(pending.outcome.borrow().as_ref(), Some(Ok(())))
        })
    }

    pub(crate) async fn wait(&self, label: &str) -> Result<(), String> {
        let pending = {
            let windows = self
                .windows
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(pending) = windows.get(label) else {
                return Ok(());
            };
            pending.clone()
        };
        pending.wait().await
    }

    pub(crate) fn remove(&self, label: &str) {
        let pending = self
            .windows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(label);
        if let Some(pending) = pending {
            pending.complete(Err(
                "window closed before geometry initialization completed".into(),
            ));
        }
    }
}

impl PendingInitialization {
    /// Wait on this ticket even if its label is removed or reused.
    pub(crate) async fn wait(&self) -> Result<(), String> {
        let mut outcome = self.outcome.subscribe();
        loop {
            if let Some(result) = outcome.borrow_and_update().clone() {
                return result;
            }
            outcome
                .changed()
                .await
                .map_err(|_| "window geometry initialization was cancelled".to_owned())?;
        }
    }

    pub(crate) fn complete(&self, result: Result<(), String>) {
        self.outcome.send_if_modified(|outcome| {
            if outcome.is_some() {
                return false;
            }
            *outcome = Some(result);
            true
        });
    }
}

#[cfg(test)]
#[path = "window_geometry_initialization_tests.rs"]
mod tests;
