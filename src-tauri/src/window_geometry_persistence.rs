//! Window-local normal bounds, expressed in the fixed host's logical pixels.
//! Native coordinate conversion and durable storage belong to the host adapters.
use std::collections::{hash_map::Entry, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LogicalBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SavedWindowState {
    pub bounds: Option<LogicalBounds>,
    pub maximized: bool,
}

#[derive(Default)]
pub(crate) struct WindowStates {
    windows: HashMap<String, TrackedWindow>,
}

struct TrackedWindow {
    bounds: Option<LogicalBounds>,
    sampling_bounds: Option<LogicalBounds>,
    skip_initial_sample: bool,
    startup_maximized: bool,
    ready: bool,
}

impl WindowStates {
    /// Retain the construction-time flag independently of later move/resize samples.
    pub fn startup_maximized(&self, label: &str) -> Option<bool> {
        self.windows
            .get(label)
            .map(|window| window.startup_maximized)
    }

    /// Seed one newly constructed window without changing other windows.
    /// A duplicate label must return false and retain the existing record.
    pub fn begin(&mut self, label: &str, saved: SavedWindowState) -> bool {
        let Entry::Vacant(entry) = self.windows.entry(label.to_owned()) else {
            return false;
        };
        entry.insert(TrackedWindow {
            bounds: saved.bounds,
            sampling_bounds: None,
            skip_initial_sample: false,
            startup_maximized: saved.maximized,
            ready: false,
        });
        true
    }

    /// Construction/restoration events must not replace the saved normal bounds.
    pub fn ready(&mut self, label: &str) {
        if let Some(window) = self.windows.get_mut(label) {
            window.ready = true;
        }
    }

    /// Enable native capture from a construction sample without saving that sample.
    pub fn ready_with_sample(&mut self, label: &str, sample: LogicalBounds) {
        if let Some(window) = self.windows.get_mut(label) {
            if !window.ready {
                window.sampling_bounds = Some(sample);
                window.skip_initial_sample = true;
                window.ready = true;
            }
        }
    }

    /// Seed a sampling origin without promoting it or suppressing the first event.
    /// X11 emits a normal configure event on first display; Wayland uses ready_with_sample.
    pub fn ready_with_origin(&mut self, label: &str, sample: LogicalBounds) {
        if let Some(window) = self.windows.get_mut(label) {
            if !window.ready {
                window.sampling_bounds = Some(sample);
                window.ready = true;
            }
        }
    }

    /// Retain a logical origin when a backend cannot report global positions.
    pub fn sampling_bounds(&self, label: &str) -> Option<LogicalBounds> {
        self.windows
            .get(label)
            .and_then(|window| window.sampling_bounds.or(window.bounds))
    }

    /// Match the fixed original move/resize guard: ignore maximized events.
    /// Fullscreen/minimized handling belongs to the native getBounds adapter.
    pub fn record(&mut self, label: &str, bounds: LogicalBounds, maximized: bool) {
        if let Some(window) = self.windows.get_mut(label) {
            if !window.ready {
                return;
            }
            if maximized {
                window.skip_initial_sample = false;
                return;
            }
            // Only unchanged startup configure events are suppressed. Restoring a
            // maximized window records its normal bounds even at the same size.
            if window.skip_initial_sample && window.sampling_bounds == Some(bounds) {
                return;
            }
            window.bounds = Some(bounds);
            if window.sampling_bounds.is_some() {
                window.sampling_bounds = Some(bounds);
            }
            window.skip_initial_sample = false;
        }
    }

    /// Called only after renderer close approval. Reading does not consume state:
    /// a cancelled close never calls this method, and a failed destroy can retry.
    pub fn for_close(&self, label: &str, maximized: bool) -> Option<SavedWindowState> {
        self.windows.get(label).map(|window| SavedWindowState {
            bounds: window.bounds,
            maximized,
        })
    }

    pub fn remove(&mut self, label: &str) {
        self.windows.remove(label);
    }
}

#[cfg(test)]
#[path = "window_geometry_persistence_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "window_geometry_linux_capture_tests.rs"]
mod linux_capture_tests;
