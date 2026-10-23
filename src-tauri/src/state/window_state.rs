//! Per-window bounds tracking and approved-close persistence use the host's state file.
use super::AppState;
use crate::{
    error::AppError,
    window_geometry::persistence::{LogicalBounds, SavedWindowState, WindowStates},
};
use std::sync::MutexGuard;

impl AppState {
    pub(crate) fn startup_window_maximized(&self, label: &str) -> Option<bool> {
        self.window_states().startup_maximized(label)
    }

    pub(crate) fn startup_window_hidden(&self, label: &str) -> bool {
        label == "main" && self.initial_window_hidden
    }

    /// Seed a new label from the current durable cache. Reject duplicate registration.
    pub(crate) fn begin_window_state(&self, label: &str) -> Result<SavedWindowState, AppError> {
        let saved = self.persisted_state().window_state.unwrap_or_default();
        if !self.window_states().begin(label, saved) {
            return Err(AppError::InvalidArgument(format!(
                "window state already registered for {label}"
            )));
        }
        Ok(saved)
    }

    /// Only construction/restoration completion enables move/resize capture.
    pub(crate) fn ready_window_state(&self, label: &str) {
        self.window_states().ready(label);
    }

    /// Arm native capture without turning constructor geometry into saved bounds.
    pub(crate) fn ready_window_state_with_sample(&self, label: &str, sample: LogicalBounds) {
        self.window_states().ready_with_sample(label, sample);
    }

    /// Arm X11 sampling without suppressing its first normal display event.
    pub(crate) fn ready_window_state_with_origin(&self, label: &str, sample: LogicalBounds) {
        self.window_states().ready_with_origin(label, sample);
    }

    /// Return the latest sampling origin for backends without global positions.
    pub(crate) fn window_sampling_bounds(&self, label: &str) -> Option<LogicalBounds> {
        self.window_states().sampling_bounds(label)
    }

    /// Record a native sample without writing to disk. Missing bounds are ignored.
    pub(crate) fn record_window_state(&self, label: &str, sample: SavedWindowState) {
        if let Some(bounds) = sample.bounds {
            self.window_states().record(label, bounds, sample.maximized);
        }
    }

    /// Called only for an approved close. Failure retains the tracker for retry.
    /// Unknown/removed labels leave the durable state unchanged.
    pub(crate) fn save_window_state(&self, label: &str, maximized: bool) -> Result<(), AppError> {
        let saved = self.window_states().for_close(label, maximized);
        if let Some(saved) = saved {
            self.update_persisted_state(|state| state.window_state = Some(saved))?;
        }
        Ok(())
    }

    /// Destroyed windows and failed construction remove only their own tracker.
    pub(crate) fn remove_window_state(&self, label: &str) {
        self.window_states().remove(label);
    }

    fn window_states(&self) -> MutexGuard<'_, WindowStates> {
        self.window_states
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
#[path = "window_state_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "window_sampling_tests.rs"]
mod sampling_tests;
