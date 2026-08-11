use std::collections::{BTreeMap, BTreeSet};

use font_kit::{properties::Style, source::SystemSource};

use crate::error::AppError;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFont {
    pub family: String,
    pub full_name: Option<String>,
    pub monospace: Option<bool>,
    pub styles: Vec<String>,
}

#[derive(Default)]
struct FontRecord {
    full_name: Option<String>,
    monospace: Option<bool>,
    styles: BTreeSet<String>,
}

pub fn list_installed_fonts() -> Result<Vec<InstalledFont>, AppError> {
    let source = SystemSource::new();
    let families = source
        .all_families()
        .map_err(|error| AppError::Io(format!("failed to enumerate installed fonts: {error}")))?;
    let mut records = families
        .into_iter()
        .map(|family| (family, FontRecord::default()))
        .collect::<BTreeMap<_, _>>();

    for handle in source
        .all_fonts()
        .map_err(|error| AppError::Io(format!("failed to enumerate installed fonts: {error}")))?
    {
        let Ok(font) = handle.load() else {
            continue;
        };
        let family = font.family_name();
        let record = records.entry(family).or_default();
        record.full_name.get_or_insert_with(|| font.full_name());
        record.monospace = Some(record.monospace.unwrap_or(false) || font.is_monospace());
        record
            .styles
            .insert(style_name(font.properties().style).into());
    }

    Ok(records
        .into_iter()
        .map(|(family, record)| InstalledFont {
            family,
            full_name: record.full_name,
            monospace: record.monospace,
            styles: record.styles.into_iter().collect(),
        })
        .collect())
}

fn style_name(style: Style) -> &'static str {
    match style {
        Style::Normal => "normal",
        Style::Italic => "italic",
        Style::Oblique => "oblique",
    }
}

#[cfg(test)]
mod tests {
    use super::style_name;
    use font_kit::properties::Style;

    #[test]
    fn maps_font_styles_to_stable_names() {
        assert_eq!(style_name(Style::Normal), "normal");
        assert_eq!(style_name(Style::Italic), "italic");
        assert_eq!(style_name(Style::Oblique), "oblique");
    }
}
