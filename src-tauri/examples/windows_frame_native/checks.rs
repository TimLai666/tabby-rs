use super::{check, Check};
use serde_json::json;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

const WS_CAPTION: u32 = 0x00c0_0000;

fn failed(name: &str, error: impl std::fmt::Display) -> Check {
    check(name, false, json!({"error": error.to_string()}))
}

pub fn run(
    first: &tauri::WebviewWindow,
    other: &tauri::WebviewWindow,
    output: &Path,
) -> Vec<Check> {
    let mut checks = Vec::new();

    let overlay = super::native::overlay_experiment(first, output);
    let overlay_ok = overlay.as_ref().is_ok_and(|detail| {
        detail["baselineBodyPixel"] == json!([19, 119, 209])
            && detail["modes"]
                .as_array()
                .is_some_and(|modes| modes.iter().any(|mode| mode["supported"] == true))
    });
    checks.push(match overlay {
        Ok(detail) => check("wry-overlay-hit-test-and-pixels", overlay_ok, detail),
        Err(error) => failed("wry-overlay-hit-test-and-pixels", error),
    });

    let initial_other = super::native::snapshot(other);
    let initial_first = super::native::snapshot(first);
    for frame in ["thin", "full", "native", "thin"] {
        let result = super::platform::set_window_frame(first, frame);
        let result_detail = match &result {
            Ok(()) => json!({"setWindowFrame": "ok"}),
            Err(error) => json!({"setWindowFrame": format!("{error:?}")}),
        };
        let first_snapshot = super::native::snapshot(first);
        let other_snapshot = super::native::snapshot(other);
        let should_have_caption = frame == "native";
        // Tao retains WS_CAPTION on top-level undecorated windows and hides the
        // title area through WM_NCCALCSIZE. Query its decoration state instead.
        let decorations_ok = first_snapshot
            .as_ref()
            .is_ok_and(|snapshot| snapshot["decorated"].as_bool() == Some(should_have_caption));
        let caption_style_present = first_snapshot
            .as_ref()
            .ok()
            .and_then(|snapshot| snapshot["style"].as_u64())
            .map(|style| style as u32 & WS_CAPTION != 0);
        let other_unchanged =
            matches!((&initial_other, &other_snapshot), (Ok(before), Ok(after)) if before == after);
        let call_ok = matches!(&result, Ok(()));
        let ok = call_ok && decorations_ok && other_unchanged;
        let detail = json!({
            "frame": frame,
            "setWindowFrame": result_detail["setWindowFrame"],
            "firstSnapshot": first_snapshot,
            "otherSnapshot": other_snapshot,
            "initialOtherSnapshot": initial_other,
            "expectedDecorations": should_have_caption,
            "captionStyleBit": format!("0x{WS_CAPTION:08x}"),
            "captionStylePresent": caption_style_present,
            "decorationsMatched": decorations_ok,
            "otherUnchanged": other_unchanged,
            "initialFirstSnapshot": initial_first
        });
        checks.push(check(
            &format!("frame-{frame}-decorations-and-isolation"),
            ok,
            detail,
        ));
    }

    let before_invalid = super::native::snapshot(first);
    let invalid_result = super::platform::set_window_frame(first, "invalid-frame-probe");
    let invalid_argument = matches!(
        &invalid_result,
        Err(super::error::AppError::InvalidArgument(_))
    );
    let after_invalid = super::native::snapshot(first);
    let invalid_unchanged =
        matches!((&before_invalid, &after_invalid), (Ok(before), Ok(after)) if before == after);
    checks.push(check(
        "invalid-frame-rejected-without-change",
        invalid_argument && invalid_unchanged,
        json!({
            "result": match invalid_result { Ok(()) => "unexpected success".to_owned(), Err(error) => format!("{error:?}") },
            "isInvalidArgument": invalid_argument,
            "before": before_invalid,
            "after": after_invalid,
            "unchanged": invalid_unchanged
        }),
    ));

    let maximize_result = first.maximize();
    let maximize_observed = if maximize_result.is_ok() {
        poll_maximized(first, true)
    } else {
        Err("maximize request failed; state was not polled".to_owned())
    };
    checks.push(check(
        "maximize-state-readback",
        maximize_result.is_ok() && maximize_observed.as_ref().is_ok_and(|observed| *observed),
        json!({"request": maximize_result.map_err(|error| error.to_string()), "observed": maximize_observed}),
    ));

    let unmaximize_result = first.unmaximize();
    let unmaximize_observed = if unmaximize_result.is_ok() {
        poll_maximized(first, false)
    } else {
        Err("unmaximize request failed; state was not polled".to_owned())
    };
    checks.push(check(
        "unmaximize-state-readback",
        unmaximize_result.is_ok() && unmaximize_observed.as_ref().is_ok_and(|observed| !*observed),
        json!({"request": unmaximize_result.map_err(|error| error.to_string()), "observed": unmaximize_observed}),
    ));

    let capture_path = output.join("wry-after-frame.png");
    match super::native::capture(first, &capture_path) {
        Ok(raster) => {
            let expected_bytes = (raster.width as usize)
                .checked_mul(raster.height as usize)
                .and_then(|pixels| pixels.checked_mul(4));
            let dimensions_ok = raster.width >= 10
                && raster.height >= 10
                && raster.width <= 4096
                && raster.height <= 4096;
            let data_size_ok = expected_bytes == Some(raster.rgba.len()) && !raster.rgba.is_empty();
            let non_blank = raster
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel[..3] != raster.rgba[..3]);
            checks.push(check(
                "wry-after-frame-capture",
                dimensions_ok && data_size_ok && non_blank,
                json!({"path": capture_path, "width": raster.width, "height": raster.height, "rgbaBytes": raster.rgba.len(), "dimensionsReasonable": dimensions_ok, "pixelDataComplete": data_size_ok, "sampleNonBlank": non_blank}),
            ));
        }
        Err(error) => checks.push(failed("wry-after-frame-capture", error)),
    }

    checks
}

fn poll_maximized(window: &tauri::WebviewWindow, expected: bool) -> Result<bool, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = super::native::snapshot(window)?;
        let maximized = snapshot["maximized"]
            .as_bool()
            .ok_or_else(|| format!("snapshot has no boolean maximized field: {snapshot}"))?;
        if maximized == expected {
            return Ok(maximized);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for maximized={expected}; last snapshot: {snapshot}"
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}
