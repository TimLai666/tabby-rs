//! Controlled DirectWrite natural-metrics diagnostic, not tooltip rendering.
use super::{check, Check};
use font_kit::{
    family_name::FamilyName,
    properties::{Properties, Stretch, Style, Weight},
    source::SystemSource,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use windows_sys::Win32::{
    Graphics::Gdi::LOGFONTW,
    UI::{HiDpi::GetDpiForSystem, WindowsAndMessaging::*},
};

fn last_error(operation: &str) -> String {
    format!("{operation}: {}", std::io::Error::last_os_error())
}

fn message_font() -> Result<LOGFONTW, String> {
    let mut metrics: NONCLIENTMETRICSW = unsafe { std::mem::zeroed() };
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    if unsafe {
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
            0,
        )
    } == 0
    {
        return Err(last_error("SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)"));
    }
    Ok(metrics.lfMessageFont)
}

// Keep failed queries explicit; an unknown setting is not a disabled setting.
fn font_smoothing() -> Value {
    fn query(action: u32) -> Result<u32, String> {
        let mut value = 0u32;
        if unsafe { SystemParametersInfoW(action, 0, (&mut value as *mut u32).cast(), 0) } == 0 {
            return Err(last_error(&format!("SystemParametersInfoW({action:#x})")));
        }
        Ok(value)
    }
    json!({
        "enabled": query(SPI_GETFONTSMOOTHING),
        "type": query(SPI_GETFONTSMOOTHINGTYPE),
        "orientation": query(SPI_GETFONTSMOOTHINGORIENTATION),
        "effectiveReferenceRuntimeSettingsVerified": false,
    })
}

fn probe_one(output: &Path, label: &str, reference_name: &str) -> Result<Value, String> {
    let original = super::paint::read_png(&output.join(reference_name))?;
    let [x, y, reference_width, reference_height] =
        super::tooltip_pixels::find(original.width, original.height, &original.rgba)?;

    let logfont = message_font()?;
    let em_px = logfont
        .lfHeight
        .checked_neg()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            format!(
                "message font lfHeight must be a negative pixel height, got {}",
                logfont.lfHeight
            )
        })?;
    let family_end = logfont
        .lfFaceName
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(logfont.lfFaceName.len());
    let family = String::from_utf16_lossy(&logfont.lfFaceName[..family_end]);
    let requested_italic = logfont.lfItalic != 0;
    let requested_style = if requested_italic {
        Style::Italic
    } else {
        Style::Normal
    };
    let requested_weight = logfont.lfWeight as f32;
    let properties = Properties {
        style: requested_style,
        weight: Weight(requested_weight),
        stretch: Stretch::NORMAL,
    };

    let source = SystemSource::new();
    let handle = source
        .select_best_match(&[FamilyName::Title(family.clone())], &properties)
        .map_err(|error| format!("select_best_match(message font): {error}"))?;
    let font = handle
        .load()
        .map_err(|error| format!("load(message font): {error}"))?;

    let matched_family = font.family_name();
    let postscript_name = font.postscript_name();
    let matched_properties = font.properties();
    let family_matches = matched_family.eq_ignore_ascii_case(&family);
    let weight_matches = matched_properties.weight.0 == requested_weight;
    let style_matches = matched_properties.style == requested_style;
    if !family_matches {
        return Err(format!(
            "message font family {matched_family:?} does not match requested {family:?}"
        ));
    }
    if !weight_matches {
        return Err(format!(
            "message font weight {} does not match requested {requested_weight}",
            matched_properties.weight.0
        ));
    }
    if !style_matches {
        return Err(format!(
            "message font style {:?} does not match requested {:?}",
            matched_properties.style, requested_style
        ));
    }

    let metrics = font.metrics();
    let units_per_em = metrics.units_per_em;
    let ascent = metrics.ascent;
    let descent = metrics.descent;
    let line_gap = metrics.line_gap;
    if units_per_em == 0 {
        return Err("message font units_per_em is zero".into());
    }
    if !ascent.is_finite() || ascent <= 0.0 {
        return Err(format!(
            "message font ascent must be positive and finite, got {ascent}"
        ));
    }
    if !descent.is_finite() || descent >= 0.0 {
        return Err(format!(
            "message font descent must be negative and finite, got {descent}"
        ));
    }
    if !line_gap.is_finite() {
        return Err(format!(
            "message font line_gap must be finite, got {line_gap}"
        ));
    }

    let scale = em_px as f64 / units_per_em as f64;
    let mut advance_sum = 0.0f64;
    let mut glyphs = Vec::with_capacity(label.len());
    let native_face = font.native_font().dwrite_font_face;
    let mut previous_glyph = None;
    let mut kerning_pairs = Vec::new();
    for character in label.chars() {
        let glyph = font
            .glyph_for_char(character)
            .ok_or_else(|| format!("message font has no glyph for {character:?}"))?;
        let advance = font
            .advance(glyph)
            .map_err(|error| format!("message font advance for {character:?}: {error}"))?;
        let advance_x = advance.x();
        if !advance_x.is_finite() || advance_x < 0.0 {
            return Err(format!(
                "message font advance for {character:?} must be finite and non-negative, got {advance_x}"
            ));
        }
        let glyph_u16 = u16::try_from(glyph)
            .map_err(|error| format!("message font glyph ID is not 16-bit: {error}"))?;
        if let Some(left) = previous_glyph {
            let adjustment = native_face
                .glyph_pair_kerning_adjustment(left, glyph_u16)
                .map_err(|hresult| format!("GetKerningPairAdjustments: {:#010x}", hresult as u32));
            kerning_pairs.push(json!({
                "leftGlyphId": left, "rightGlyphId": glyph_u16,
                "adjustmentDesignUnits": adjustment,
            }));
        }
        previous_glyph = Some(glyph_u16);
        advance_sum += advance_x as f64;
        glyphs.push(json!({"char": character.to_string(), "glyphId": glyph,
            "advance": advance_x, "pixelAdvance": advance_x as f64 * scale}));
    }

    // PlatformFontSkia rounds these up; retain the fractional values for diagnosis.
    let text_width_float = advance_sum * scale;
    let line_height_float = (ascent - descent) as f64 * scale;
    let baseline_float = ascent as f64 * scale;
    if !text_width_float.is_finite()
        || !line_height_float.is_finite()
        || !baseline_float.is_finite()
    {
        return Err("computed tooltip dimensions are not finite".into());
    }
    if text_width_float < 0.0 || line_height_float < 0.0 || baseline_float < 0.0 {
        return Err("computed tooltip dimensions are negative".into());
    }
    let text_width = text_width_float.ceil() as u32;
    let line_height = line_height_float.ceil() as u32;
    let baseline = baseline_float.ceil() as u32;
    let width = text_width.checked_add(16).ok_or("tooltip width overflow")?;
    let height = line_height
        .checked_add(9)
        .ok_or("tooltip height overflow")?;
    if !(18..=800).contains(&width) || !(10..=96).contains(&height) {
        return Err(format!(
            "computed tooltip dimensions exceed diagnostic bounds: {width}x{height}"
        ));
    }
    let dimensions_match = width == reference_width && height == reference_height;

    let font_data = font
        .copy_font_data()
        .filter(|bytes| !bytes.is_empty())
        .ok_or("message font data is unavailable for fingerprinting")?;
    let font_data_detail = json!({
        "byteLength": font_data.len(),
        "sha256": format!("{:x}", Sha256::digest(font_data.as_slice())),
    });
    let gasp_detail = match font.load_font_table(u32::from_be_bytes(*b"gasp")) {
        Some(bytes) if bytes.len() > 4096 => {
            return Err(format!(
                "message font gasp table is {} bytes, exceeding the 4096-byte diagnostic limit",
                bytes.len()
            ));
        }
        Some(bytes) => json!({"byteLength": bytes.len(), "hex": hex::encode(&bytes[..])}),
        None => Value::Null,
    };

    Ok(json!({
        "diagnosticOnly": true,
        "productionTooltipVerified": false,
        "pixelsVerified": false,
        "shapingVerified": false,
        "shapingSemantics":
            "textWidth is the sum of unshaped natural glyph advances scaled to pixels; it is not a complete shaping proof",
        "label": label,
        "systemDpi": unsafe { GetDpiForSystem() },
        "fontSmoothing": font_smoothing(),
        "legacyKerning": {
            "api": "IDWriteFontFace1::GetKerningPairAdjustments",
            "hasKerningPairs": native_face.has_kerning_pairs(),
            "zeroMayIndicateUnavailableFontFace1": true,
            "pairs": kerning_pairs,
            "shapingVerified": false,
        },
        "reference": reference_name,
        "referenceTooltip": [x, y, reference_width, reference_height],
        "fontFamily": {
            "requested": {
                "family": family,
                "weight": requested_weight,
                "italic": requested_italic,
                "style": format!("{:?}", requested_style),
            },
            "matched": {
                "family": matched_family,
                "postscriptName": postscript_name,
                "weight": matched_properties.weight.0,
                "style": format!("{:?}", matched_properties.style),
                "stretch": matched_properties.stretch.0,
            },
            "familyMatchesAsciiCaseInsensitive": family_matches,
            "weightMatches": weight_matches,
            "styleMatches": style_matches,
        },
        "rawMetrics": {
            "unitsPerEm": units_per_em,
            "ascent": ascent,
            "descent": descent,
            "lineGap": line_gap,
        },
        "convertedFloats": {
            "emPx": em_px,
            "scale": scale,
            "advanceSumDesignUnits": advance_sum,
            "textWidth": text_width_float,
            "lineHeight": line_height_float,
            "baseline": baseline_float,
        },
        "baselinePx": baseline,
        "lineHeightPx": line_height,
        "textWidthPx": text_width,
        "glyphs": glyphs,
        "dimensions": {"width": width, "height": height},
        "dimensionsMatch": dimensions_match,
        "fontData": font_data_detail,
        "gaspTable": gasp_detail,
    }))
}

pub fn probe(output: &Path) -> Vec<Check> {
    [
        ("Maximize", "electron-thin-hover-max-tooltip.png"),
        ("Restore", "electron-thin-hover-restore-tooltip.png"),
    ]
    .into_iter()
    .map(|(label, reference)| {
        let result = probe_one(output, label, reference);
        let matched = result
            .as_ref()
            .is_ok_and(|detail| detail["dimensionsMatch"] == true);
        check(
            &format!(
                "dwrite-unshaped-tooltip-{}-dimensions",
                label.to_ascii_lowercase()
            ),
            matched,
            json!(result),
        )
    })
    .collect()
}
