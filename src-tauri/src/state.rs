use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};

use crate::{
    error::AppError,
    identity::AppPaths,
    launch::LaunchContext,
    plugins::npm::OperationManager,
    storage::{
        paths::StoragePaths,
        state_file::{save_state, TabbyRsState},
    },
    update::service::UpdateManager,
};

pub struct AppState {
    next_window_id: AtomicU64,
    initial_launch: Mutex<Option<LaunchContext>>,
    storage_lock: Mutex<()>,
    plugin_operations: OperationManager,
    paths: AppPaths,
    persisted_state: Mutex<TabbyRsState>,
    update_manager: Arc<UpdateManager>,
}

impl AppState {
    pub fn new(
        paths: AppPaths,
        initial_launch: LaunchContext,
        persisted_state: TabbyRsState,
    ) -> Self {
        Self {
            next_window_id: AtomicU64::new(0),
            initial_launch: Mutex::new(Some(initial_launch)),
            storage_lock: Mutex::new(()),
            plugin_operations: OperationManager::default(),
            paths,
            persisted_state: Mutex::new(persisted_state),
            update_manager: Arc::new(UpdateManager::default()),
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
