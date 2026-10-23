mod backup;
mod window_frame;
mod window_state;

#[cfg(test)]
#[path = "state/persistence_tests.rs"]
mod persistence_tests;

use std::{
    collections::BTreeMap,
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
    },
    update::service::UpdateManager,
    window_geometry::persistence::WindowStates,
};

pub struct AppState {
    next_window_id: AtomicU64,
    force_quit_requested: AtomicBool,
    launches: LaunchQueue,
    initial_window_hidden: bool,
    storage_lock: Mutex<()>,
    window_operations: tokio::sync::Mutex<()>,
    window_states: Mutex<WindowStates>,
    plugin_operations: OperationManager,
    paths: AppPaths,
    persisted_state: Mutex<TabbyRsState>,
    update_manager: Arc<UpdateManager>,
    startup_window_frame: window_frame::StartupWindowFrame,
}

impl AppState {
    pub fn new(
        paths: AppPaths,
        initial_launch: LaunchContext,
        persisted_state: TabbyRsState,
    ) -> Self {
        let initial_window_hidden = initial_launch.request.argv.hidden;
        let launches = LaunchQueue::default();
        launches.push("main", initial_launch.for_new_window());
        Self {
            next_window_id: AtomicU64::new(0),
            force_quit_requested: AtomicBool::new(false),
            launches,
            initial_window_hidden,
            storage_lock: Mutex::new(()),
            window_operations: tokio::sync::Mutex::new(()),
            window_states: Mutex::new(WindowStates::default()),
            plugin_operations: OperationManager::default(),
            paths,
            persisted_state: Mutex::new(persisted_state),
            update_manager: Arc::new(UpdateManager::default()),
            startup_window_frame: window_frame::StartupWindowFrame::default(),
        }
    }

    pub fn next_window_id(&self) -> u64 {
        self.next_window_id.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn startup_window_frame(&self, config: &BTreeMap<String, serde_json::Value>) -> &str {
        self.startup_window_frame.get(config)
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

    pub(crate) async fn lock_window_operations(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.window_operations.lock().await
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
        let mut candidate = state.clone();
        update(&mut candidate);
        let storage_paths = StoragePaths::from_app_paths(&self.paths);
        storage_paths.ensure_layout()?;
        save_state(storage_paths.state_file(), &candidate)?;
        *state = candidate;
        Ok(state.clone())
    }
}
