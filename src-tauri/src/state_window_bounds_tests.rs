use super::AppState;
use crate::{
    identity::AppPaths,
    launch::{LaunchContext, LaunchRequest},
    storage::{state_file::TabbyRsState, window_state::WindowBoundaries},
};

fn state() -> AppState {
    let temp = tempfile::tempdir().unwrap();
    AppState::new(
        AppPaths::for_tests(temp.path().join("data")),
        LaunchContext {
            request: LaunchRequest::default(),
            cwd: String::new(),
            second_instance: false,
            parse_error: None,
        },
        TabbyRsState::default(),
    )
}

fn bounds(x: f64) -> WindowBoundaries {
    WindowBoundaries {
        x,
        y: 20.0,
        width: 800.0,
        height: 600.0,
    }
}

#[test]
fn remembers_bounds_per_window_label() {
    let state = state();
    assert_eq!(state.remembered_window_bounds("main"), None);
    state.remember_window_bounds("main", bounds(1.0));
    state.remember_window_bounds("window-1", bounds(2.0));
    assert_eq!(state.remembered_window_bounds("main"), Some(bounds(1.0)));
    assert_eq!(
        state.remembered_window_bounds("window-1"),
        Some(bounds(2.0))
    );
}

#[test]
fn later_bounds_replace_earlier_ones() {
    let state = state();
    state.remember_window_bounds("main", bounds(1.0));
    state.remember_window_bounds("main", bounds(3.0));
    assert_eq!(state.remembered_window_bounds("main"), Some(bounds(3.0)));
}

#[test]
fn forgetting_removes_only_that_window() {
    let state = state();
    state.remember_window_bounds("main", bounds(1.0));
    state.remember_window_bounds("window-1", bounds(2.0));
    state.forget_window_bounds("main");
    assert_eq!(state.remembered_window_bounds("main"), None);
    assert_eq!(
        state.remembered_window_bounds("window-1"),
        Some(bounds(2.0))
    );
    state.forget_window_bounds("missing");
}
