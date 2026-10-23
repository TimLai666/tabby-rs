//! Production capture/restore methods with real Wry events and AppKit geometry.
//! Only the AppState boundary is replaced by the actual in-memory tracker.
use crate::{
    error::AppError,
    platform::window_state_macos::restore_bounds,
    window_geometry::persistence::{LogicalBounds, SavedWindowState, WindowStates},
};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

pub static CHECKS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct CaptureState {
    seed: SavedWindowState,
    windows: Mutex<WindowStates>,
    hidden: bool,
    config: BTreeMap<String, serde_json::Value>,
    window_operations: tokio::sync::Mutex<()>,
}

impl CaptureState {
    pub(crate) fn new(seed: SavedWindowState, hidden: bool) -> Self {
        let mut config = BTreeMap::new();
        config.insert(
            "appearance".into(),
            serde_json::json!({"frame": "full", "colorSchemeMode": "dark"}),
        );
        Self {
            seed,
            windows: Mutex::new(WindowStates::default()),
            hidden,
            config,
            window_operations: tokio::sync::Mutex::new(()),
        }
    }

    pub(crate) fn startup_window_config(
        &self,
    ) -> Result<BTreeMap<String, serde_json::Value>, AppError> {
        Ok(self.config.clone())
    }

    pub(crate) fn startup_window_frame(
        &self,
        _config: &BTreeMap<String, serde_json::Value>,
    ) -> &str {
        "full"
    }

    pub(crate) fn startup_window_maximized(&self, label: &str) -> Option<bool> {
        self.windows.lock().unwrap().startup_maximized(label)
    }

    pub(crate) fn startup_window_hidden(&self, _label: &str) -> bool {
        self.hidden
    }

    pub(crate) fn launches(&self) -> FixtureLaunches {
        FixtureLaunches
    }

    pub(crate) async fn lock_window_operations(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.window_operations.lock().await
    }

    pub(crate) fn begin_window_state(&self, label: &str) -> Result<SavedWindowState, AppError> {
        if !self.windows.lock().unwrap().begin(label, self.seed) {
            return Err(AppError::InvalidArgument("duplicate fixture label".into()));
        }
        Ok(self.seed)
    }
    pub(crate) fn ready_window_state(&self, label: &str) {
        self.windows.lock().unwrap().ready(label);
    }
    pub(crate) fn record_window_state(&self, label: &str, sample: SavedWindowState) {
        if let Some(bounds) = sample.bounds {
            self.windows
                .lock()
                .unwrap()
                .record(label, bounds, sample.maximized);
        }
    }
    pub(crate) fn remove_window_state(&self, label: &str) {
        self.windows.lock().unwrap().remove(label);
    }
    fn cached(&self, label: &str) -> Option<SavedWindowState> {
        self.windows.lock().unwrap().for_close(label, false)
    }
}

pub(crate) struct FixtureLaunches;

impl FixtureLaunches {
    pub(crate) fn is_closing(&self, _label: &str) -> bool {
        false
    }
}

pub fn run_checks(first: &tauri::WebviewWindow, second: &tauri::WebviewWindow) -> Vec<String> {
    let mut failures = Vec::new();
    for (window, peer) in [(first, second), (second, first)] {
        CHECKS.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = check(window, peer) {
            failures.push(format!(
                "{} native capture ordering: {error}",
                window.label()
            ));
        }
    }
    failures
}

fn check(window: &tauri::WebviewWindow, peer: &tauri::WebviewWindow) -> Result<(), String> {
    let original = crate::macos_saved::native_bounds(window)?;
    let peer_before = crate::macos_saved::native_bounds(peer)?;
    let seed = SavedWindowState {
        bounds: Some(LogicalBounds {
            x: 5000,
            y: 120,
            width: 700,
            height: 450,
        }),
        maximized: false,
    };
    let state = Arc::new(CaptureState::new(seed, false));
    let owner = window.clone();
    let observed = state.clone();
    let errors = Arc::new(Mutex::new(Vec::new()));
    let event_errors = errors.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
        ) {
            if let Err(error) = crate::window_persistence_macos::capture_window(&owner, &observed) {
                event_errors.lock().unwrap().push(error.to_string());
            }
        }
    });
    let result = (|| {
        crate::window_persistence_macos::restore_window(window, &state)
            .map_err(|error| error.to_string())?;
        tauri::async_runtime::block_on(crate::window_persistence_macos::enable_capture(
            window, &state,
        ))
        .map_err(|error| error.to_string())?;
        if state.cached(window.label()) != Some(seed) {
            return Err(format!(
                "construction events replaced saved bounds: {:?}",
                state.cached(window.label())
            ));
        }
        let restored = crate::macos_saved::native_bounds(window)?;
        let moved = LogicalBounds {
            x: restored.x + 17,
            y: restored.y + 11,
            width: restored.width + 23,
            height: restored.height + 19,
        };
        restore_bounds(window, moved).map_err(|error| error.to_string())?;
        tauri::async_runtime::block_on(crate::platform::wait_for_window_operations(window))
            .map_err(|error| error.to_string())?;
        if state
            .cached(window.label())
            .and_then(|sample| sample.bounds)
            != Some(moved)
        {
            return Err(format!(
                "post-ready native events did not retain {moved:?}: {:?}",
                state.cached(window.label())
            ));
        }
        if crate::macos_saved::native_bounds(peer)? != peer_before {
            return Err("capture changed the other window".into());
        }
        if !errors.lock().unwrap().is_empty() {
            return Err(format!(
                "native capture errors: {:?}",
                errors.lock().unwrap()
            ));
        }
        Ok(())
    })();
    let cleanup = restore_bounds(window, original).map_err(|error| error.to_string());
    result?;
    cleanup?;
    if crate::macos_saved::native_bounds(window)? != original {
        return Err("could not restore owned fixture bounds".into());
    }
    Ok(())
}
