use std::{collections::BTreeMap, sync::OnceLock};

// Chromium 140.0.7339.41 translations and license are stored alongside this file.
pub(super) fn empty_submenu_label(locales: &[String]) -> &'static str {
    static LABELS: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let labels = LABELS.get_or_init(|| {
        serde_json::from_str::<BTreeMap<String, String>>(include_str!("empty_submenu_labels.json"))
            .expect("checked-in Chromium menu translations must be valid JSON")
            .into_iter()
            .map(|(locale, label)| (locale.to_lowercase(), label))
            .collect()
    });
    for locale in locales {
        let normalized = locale.replace('_', "-").to_lowercase();
        let mut candidate = normalized.as_str();
        if candidate.starts_with("zh-hant") {
            return &labels["zh-tw"];
        }
        if candidate.starts_with("zh-hans") {
            return &labels["zh-cn"];
        }
        loop {
            let alias = match candidate {
                "nb" => "no",
                "iw" => "he",
                "in" => "id",
                "tl" => "fil",
                "zh" => "zh-cn",
                "pt" => "pt-br",
                _ => candidate,
            };
            if let Some(label) = labels.get(alias) {
                return label;
            }
            let Some((parent, _)) = candidate.rsplit_once('-') else {
                break;
            };
            candidate = parent;
        }
    }
    &labels["en"]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_native_language_tags_and_fallbacks() {
        for (locale, expected) in [
            ("en-US", "(empty)"),
            ("zh-Hant-TW", "(空白)"),
            ("zh_HK", "(空白)"),
            ("zh-Hans-CN", "（空）"),
            ("ja-JP", "(なし)"),
            ("nb-NO", "(tom)"),
            ("sr-Latn-RS", "(prazno)"),
            ("unknown", "(empty)"),
        ] {
            assert_eq!(empty_submenu_label(&[locale.into()]), expected);
        }
        assert_eq!(
            empty_submenu_label(&["unknown".into(), "ja".into()]),
            "(なし)"
        );
    }
    #[test]
    fn includes_every_pinned_upstream_translation() {
        let labels: BTreeMap<String, String> =
            serde_json::from_str(include_str!("empty_submenu_labels.json")).unwrap();
        assert_eq!(labels.len(), 82);
        assert!(labels
            .values()
            .all(|value| !value.is_empty() && !value.contains('\0')));
    }
}
