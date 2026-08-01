use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, MutexGuard,
};

use crate::{identity::AppPaths, launch::LaunchContext};

pub struct AppState {
    next_window_id: AtomicU64,
    initial_launch: Mutex<Option<LaunchContext>>,
    storage_lock: Mutex<()>,
    paths: AppPaths,
}

impl AppState {
    pub fn new(paths: AppPaths, initial_launch: LaunchContext) -> Self {
        Self {
            next_window_id: AtomicU64::new(0),
            initial_launch: Mutex::new(Some(initial_launch)),
            storage_lock: Mutex::new(()),
            paths,
        }
    }

    pub fn next_window_id(&self) -> u64 {
        self.next_window_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn take_initial_launch(&self) -> Option<LaunchContext> {
        self.initial_launch
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub fn lock_storage(&self) -> MutexGuard<'_, ()> {
        self.storage_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }
}
