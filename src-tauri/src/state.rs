use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct AppState {
    next_window_id: AtomicU64,
}

impl AppState {
    pub fn next_window_id(&self) -> u64 {
        self.next_window_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}
