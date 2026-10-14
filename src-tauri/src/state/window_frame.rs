use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Default)]
pub(super) struct StartupWindowFrame {
    value: OnceLock<String>,
}

impl StartupWindowFrame {
    pub(super) fn get(&self, config: &BTreeMap<String, serde_json::Value>) -> &str {
        self.value
            .get_or_init(|| {
                let frame = config
                    .get("appearance")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|appearance| appearance.get("frame"))
                    .and_then(serde_json::Value::as_str);
                match frame {
                    Some("native") => "native",
                    Some("full") => "full",
                    _ => "thin",
                }
                .to_owned()
            })
            .as_str()
    }
}

#[cfg(test)]
#[path = "window_frame_tests.rs"]
mod tests;
