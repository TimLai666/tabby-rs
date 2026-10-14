use super::StartupWindowFrame;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Barrier},
    thread,
};

fn config_with_frame(frame: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([("appearance".to_owned(), json!({ "frame": frame }))])
}

#[test]
fn normalizes_native_full_and_thin_startup_frames() {
    for expected in ["native", "full", "thin"] {
        let cache = StartupWindowFrame::default();
        let config = config_with_frame(json!(expected));
        assert_eq!(cache.get(&config), expected);
    }
}

#[test]
fn missing_null_array_and_invalid_frames_fall_back_to_thin() {
    let configs = [
        BTreeMap::new(),
        BTreeMap::from([("appearance".to_owned(), Value::Null)]),
        BTreeMap::from([("appearance".to_owned(), json!([]))]),
        config_with_frame(Value::Null),
        config_with_frame(json!([])),
        config_with_frame(json!("invalid")),
    ];
    for config in configs {
        let cache = StartupWindowFrame::default();
        assert_eq!(cache.get(&config), "thin", "config={config:?}");
    }
}

#[test]
fn encrypted_config_without_raw_appearance_falls_back_to_thin() {
    let config = BTreeMap::from([(
        "encrypted".to_owned(),
        json!({ "version": 1, "ciphertext": "opaque" }),
    )]);
    assert_eq!(StartupWindowFrame::default().get(&config), "thin");
}

#[test]
fn first_read_is_stable_but_a_new_process_cache_reads_updated_config() {
    let cache = StartupWindowFrame::default();
    let original = config_with_frame(json!("native"));
    let updated = config_with_frame(json!("full"));
    assert_eq!(cache.get(&original), "native");
    assert_eq!(cache.get(&updated), "native");

    let restarted_process = StartupWindowFrame::default();
    assert_eq!(restarted_process.get(&updated), "full");
}

#[test]
fn concurrent_first_reads_share_one_winning_startup_frame() {
    const READERS: usize = 16;
    let cache = Arc::new(StartupWindowFrame::default());
    let barrier = Arc::new(Barrier::new(READERS));
    let handles: Vec<_> = (0..READERS)
        .map(|index| {
            let cache = Arc::clone(&cache);
            let barrier = Arc::clone(&barrier);
            let frame = ["native", "full", "thin"][index % 3];
            let config = config_with_frame(json!(frame));
            thread::spawn(move || {
                barrier.wait();
                cache.get(&config).to_owned()
            })
        })
        .collect();
    let results: Vec<String> = handles
        .into_iter()
        .map(|handle| handle.join().expect("frame reader thread"))
        .collect();
    let winners: HashSet<&str> = results.iter().map(String::as_str).collect();
    assert_eq!(
        winners.len(),
        1,
        "all readers must observe one winner: {results:?}"
    );
    assert!(matches!(results[0].as_str(), "native" | "full" | "thin"));
}
