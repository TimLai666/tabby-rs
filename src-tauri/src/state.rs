mod window_frame;

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use crate::{
    error::AppError,
    identity::AppPaths,
    launch::{LaunchContext, LaunchQueue},
    plugins::npm::OperationManager,
    storage::{
        paths::StoragePaths,
        state_file::{save_state, TabbyRsState},
        window_state::WindowBoundaries,
    },
    update::service::UpdateManager,
};

pub struct AppState {
    next_window_id: AtomicU64,
    force_quit_requested: AtomicBool,
    launches: LaunchQueue,
    storage_lock: Mutex<()>,
    plugin_operations: OperationManager,
    paths: AppPaths,
    persisted_state: Mutex<TabbyRsState>,
    update_manager: Arc<UpdateManager>,
    startup_window_frame: window_frame::StartupWindowFrame,
    window_bounds: Mutex<HashMap<String, WindowBoundaries>>,
    pending_maximize: Mutex<std::collections::HashSet<String>>,
}

impl AppState {
    pub fn new(
        paths: AppPaths,
        initial_launch: LaunchContext,
        persisted_state: TabbyRsState,
    ) -> Self {
        let launches = LaunchQueue::default();
        launches.push("main", initial_launch.for_new_window());
        Self {
            next_window_id: AtomicU64::new(0),
            force_quit_requested: AtomicBool::new(false),
            launches,
            storage_lock: Mutex::new(()),
            plugin_operations: OperationManager::default(),
            paths,
            persisted_state: Mutex::new(persisted_state),
            update_manager: Arc::new(UpdateManager::default()),
            startup_window_frame: window_frame::StartupWindowFrame::default(),
            window_bounds: Mutex::new(HashMap::new()),
            pending_maximize: Mutex::new(std::collections::HashSet::new()),
        }
    }

    pub fn next_window_id(&self) -> u64 {
        self.next_window_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn startup_window_frame(&self, config: &BTreeMap<String, serde_json::Value>) -> &str {
        self.startup_window_frame.get(config)
    }

    /// Remembers the latest unmaximized placement of a window, like upstream's
    /// `resize`/`move` handlers. Later calls for the same label replace it.
    pub fn remember_window_bounds(&self, label: &str, bounds: WindowBoundaries) {
        self.window_bounds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(label.to_owned(), bounds);
    }

    /// The last remembered unmaximized placement of a window, if any.
    pub fn remembered_window_bounds(&self, label: &str) -> Option<WindowBoundaries> {
        self.window_bounds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(label)
            .copied()
    }

    /// Drops the remembered placement after the window is destroyed.
    pub fn forget_window_bounds(&self, label: &str) {
        self.window_bounds
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(label);
        self.pending_maximize
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(label);
    }

    /// Records that a restored window must be maximized once it is shown.
    pub fn set_pending_maximize(&self, label: &str) {
        self.pending_maximize
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(label.to_owned());
    }

    /// Clears and returns whether a maximize was pending for the window.
    pub fn take_pending_maximize(&self, label: &str) -> bool {
        self.pending_maximize
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(label)
    }

    pub fn request_forced_quit(&self) {
        self.force_quit_requested.store(true, Ordering::Release);
    }

    pub fn is_force_quit_requested(&self) -> bool {
        self.force_quit_requested.load(Ordering::Acquire)
    }

    pub fn launches(&self) -> &LaunchQueue {
        &self.launches
    }

    pub fn lock_storage(&self) -> MutexGuard<'_, ()> {
        self.storage_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn plugin_operations(&self) -> &OperationManager {
        &self.plugin_operations
    }

    pub fn update_manager(&self) -> &Arc<UpdateManager> {
        &self.update_manager
    }

    pub fn persisted_state(&self) -> TabbyRsState {
        self.persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn update_persisted_state<F>(&self, update: F) -> Result<TabbyRsState, AppError>
    where
        F: FnOnce(&mut TabbyRsState),
    {
        let _guard = self.lock_storage();
        let mut state = self
            .persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state);
        let storage_paths = StoragePaths::from_app_paths(&self.paths);
        storage_paths.ensure_layout()?;
        save_state(storage_paths.state_file(), &state)?;
        Ok(state.clone())
    }
}

#[cfg(test)]
#[path = "state_window_bounds_tests.rs"]
mod window_bounds_tests;
