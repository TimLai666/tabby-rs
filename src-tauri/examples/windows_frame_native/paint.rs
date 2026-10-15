//! Production painter inside the owned WebView; no production lifecycle acceptance.
use super::{check, native::Raster, Check};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
    sync::mpsc::sync_channel,
    thread,
    time::{Duration, Instant},
};

pub struct Payload {
    script: String,
    source_sha256: String,
    script_sha256: String,
}

impl Payload {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut script = String::new();
        File::open(path)
            .map_err(fail)?
            .take(65537)
            .read_to_string(&mut script)
            .map_err(fail)?;
        if script.is_empty() || script.len() > 65536 {
            return Err("caption script must contain 1 through 65536 UTF-8 bytes".into());
        }
        Ok(Self {
            source_sha256: format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../../tabby-tauri/src/components/windowsCaption.ts"
                ))
            ),
            script_sha256: format!("{:x}", Sha256::digest(script.as_bytes())),
            script,
        })
    }

    pub fn install(&self, window: &tauri::WebviewWindow) -> Result<Value, String> {
        let result = evaluate(
            window,
            &format!(
                r#"(() => {{ try {{
            {}
            return {{sourceSha256:window.__tabbyCaptionFixture.sourceSha256,initial:window.__tabbyCaptionFixture.read()}};
        }} catch(error) {{ return {{error:String(error)}}; }} }})()"#,
                self.script
            ),
        )?;
        if result["sourceSha256"] != self.source_sha256 || !result["initial"].is_null() {
            return Err(format!(
                "caption payload is stale or painted during installation: {result}"
            ));
        }
        Ok(
            json!({"sourceSha256":self.source_sha256,"scriptSha256":self.script_sha256,"initial":result}),
        )
    }

    pub fn probe(
        &self,
        window: &tauri::WebviewWindow,
        snapshot: &Value,
        reference: &Value,
        output: &Path,
        mode: &str,
    ) -> Vec<Check> {
        let (reference_name, reference_state, compare_full) = match mode {
            "normal" | "restored" => ("electron-thin.png", "restored", true),
            "hover-max" => ("electron-thin-hover-max.png", "restored", true),
            "hover-restore" => ("electron-thin-hover-restore.png", "maximized", false),
            _ => {
                return vec![check(
                    "caption-paint-invalid-mode",
                    false,
                    json!({"mode":mode}),
                )]
            }
        };
        let captured = self.capture(
            window,
            snapshot,
            &reference["caption"][reference_state],
            output,
            reference_name,
            &format!("caption-paint-{mode}.png"),
        );
        match captured {
            Ok(detail) => {
                let mut checks = vec![check(
                    &format!("caption-paint-{mode}-caption-pixels"),
                    detail["caption"]["differentPixels"] == 0,
                    detail.clone(),
                )];
                if compare_full {
                    checks.push(check(
                        &format!("caption-paint-{mode}-full-image"),
                        detail["fullImage"]["differentPixels"] == 0,
                        detail,
                    ));
                }
                checks
            }
            Err(error) => vec![check(
                &format!("caption-paint-{mode}"),
                false,
                json!({"error":error}),
            )],
        }
    }

    fn capture(
        &self,
        window: &tauri::WebviewWindow,
        snapshot: &Value,
        reference_geometry: &Value,
        output: &Path,
        reference_name: &str,
        candidate_name: &str,
    ) -> Result<Value, String> {
        let started = evaluate(
            window,
            &format!(
                r#"(() => {{ try {{ return window.__tabbyCaptionFixture.paint({}); }} catch(error) {{ return {{error:String(error)}}; }} }})()"#,
                snapshot
            ),
        )?;
        let token = started["token"]
            .as_u64()
            .filter(|token| *token > 0)
            .ok_or("paint did not return a token")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let readback = loop {
            let state = evaluate(window, "window.__tabbyCaptionFixture.read()")?;
            if state["token"] != token || state["revision"] != snapshot["revision"] {
                return Err(format!(
                    "caption paint acknowledgement belongs to another state: {state}"
                ));
            }
            if state["ready"] == true {
                break state;
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "caption painter did not finish two frames: {state}"
                ));
            }
            thread::sleep(Duration::from_millis(20));
        };
        let dpr = readback["devicePixelRatio"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("invalid WebView DPR")?;
        if readback["sourceSha256"] != self.source_sha256
            || readback["svgCount"] != 1
            || readback["interactiveCount"] != 0
        {
            return Err(format!("caption painter source or DOM differs: {readback}"));
        }
        for (css_key, native_key) in [
            ("x", "x"),
            ("y", "y"),
            ("width", "width"),
            ("height", "height"),
        ] {
            // DOM bounds can be quantized at fractional DPR. Preserve them in the report;
            // validate the declared mapping here and use exact pixels for the rendered verdict.
            let actual = readback["declaredBounds"][css_key]
                .as_f64()
                .ok_or("painted CSS bound is missing")?;
            let expected = snapshot["layout"]["clip"][native_key]
                .as_f64()
                .ok_or("native caption bound is missing")?
                / dpr;
            if !actual.is_finite() || (actual - expected).abs() > 0.000001 {
                return Err(format!(
                    "native-to-WebView geometry differs: {readback}; native={snapshot}"
                ));
            }
        }
        let candidate = super::native::capture(window, &output.join(candidate_name))?;
        let original = read_png(&output.join(reference_name))?;
        let candidate_rect = Rect::read(&snapshot["layout"]["clip"])?;
        let reference_rect = Rect::read(&reference_geometry["nativeButtonClip"])?;
        let caption = compare(&candidate, candidate_rect, &original, reference_rect)?;
        let full_image = if candidate.width == original.width && candidate.height == original.height
        {
            let whole = Rect {
                x: 0,
                y: 0,
                width: candidate.width,
                height: candidate.height,
            };
            compare(&candidate, whole, &original, whole)?
        } else {
            Value::Null
        };
        Ok(
            json!({"nativeSnapshot":snapshot,"paintReadback":readback,"sourceSha256":self.source_sha256,
            "scriptSha256":self.script_sha256,"reference":reference_name,"candidate":candidate_name,
            "candidateDimensions":[candidate.width,candidate.height],"referenceDimensions":[original.width,original.height],
            "candidateCaption":candidate_rect,"referenceCaption":reference_rect,"caption":caption,"fullImage":full_image,
            "fullImageCompared":!full_image.is_null(),"productionAppVerified":false,"win11SnapVerified":false}),
        )
    }
}

fn fail(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn evaluate(window: &tauri::WebviewWindow, script: &str) -> Result<Value, String> {
    let (tx, rx) = sync_channel(1);
    window
        .eval_with_callback(script, move |raw| {
            let _ = tx.try_send(raw);
        })
        .map_err(fail)?;
    let raw = rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|error| format!("caption script callback: {error}"))?;
    if raw.len() > 32768 {
        return Err("caption script readback is excessive".into());
    }
    let result: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("caption script JSON: {error}: {raw}"))?;
    if !result["error"].is_null() {
        return Err(format!("caption script failed: {result}"));
    }
    Ok(result)
}

#[derive(Clone, Copy, serde::Serialize)]
pub(super) struct Rect {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}
impl Rect {
    fn read(value: &Value) -> Result<Self, String> {
        let read = |key: &str| {
            value[key]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 4096.0)
                .ok_or_else(|| format!("caption crop is not a bounded physical rectangle: {value}"))
        };
        let x = read("x")?;
        let y = read("y")?;
        let right = x + read("width")?;
        let bottom = y + read("height")?;
        if right > 4096.0 || bottom > 4096.0 {
            return Err("caption crop exceeds physical bounds".into());
        }
        // Integer hit-test coordinates in [edge, end) run from ceil(edge) to ceil(end).
        Ok(Self {
            x: x.ceil() as u32,
            y: y.ceil() as u32,
            width: (right.ceil() - x.ceil()) as u32,
            height: (bottom.ceil() - y.ceil()) as u32,
        })
    }
    fn valid(self, image: &Raster) -> bool {
        self.width > 0
            && self.height > 0
            && self
                .x
                .checked_add(self.width)
                .is_some_and(|x| x <= image.width)
            && self
                .y
                .checked_add(self.height)
                .is_some_and(|y| y <= image.height)
    }
}

pub(super) fn read_png(path: &Path) -> Result<Raster, String> {
    let file = BufReader::new(File::open(path).map_err(fail)?);
    let decoder = png::Decoder::new_with_limits(
        file,
        png::Limits {
            bytes: 64 * 1024 * 1024,
        },
    );
    let mut reader = decoder.read_info().map_err(fail)?;
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 64 * 1024 * 1024)
        .ok_or("reference PNG is excessive")?;
    let mut rgba = vec![0; size];
    let info = reader.next_frame(&mut rgba).map_err(fail)?;
    if info.color_type != png::ColorType::Rgba
        || info.bit_depth != png::BitDepth::Eight
        || info.width == 0
        || info.height == 0
        || info.width > 4096
        || info.height > 4096
        || info.buffer_size() != info.width as usize * info.height as usize * 4
    {
        return Err("reference PNG must be bounded RGBA8".into());
    }
    rgba.truncate(info.buffer_size());
    Ok(Raster {
        width: info.width,
        height: info.height,
        rgba,
    })
}

pub(super) fn compare(a: &Raster, ar: Rect, b: &Raster, br: Rect) -> Result<Value, String> {
    if !ar.valid(a) || !br.valid(b) || ar.width != br.width || ar.height != br.height {
        return Err(format!(
            "caption crop dimensions differ or exceed their captures: {}",
            json!({"candidate":ar,"reference":br})
        ));
    }
    let mut different = 0u64;
    let mut bounds = None::<[u32; 4]>;
    for y in 0..ar.height {
        for x in 0..ar.width {
            let ai = (((ar.y + y) * a.width + ar.x + x) * 4) as usize;
            let bi = (((br.y + y) * b.width + br.x + x) * 4) as usize;
            if a.rgba[ai..ai + 4] != b.rgba[bi..bi + 4] {
                different += 1;
                match &mut bounds {
                    Some(bounds) => {
                        bounds[0] = bounds[0].min(x);
                        bounds[1] = bounds[1].min(y);
                        bounds[2] = bounds[2].max(x);
                        bounds[3] = bounds[3].max(y);
                    }
                    None => bounds = Some([x, y, x, y]),
                }
            }
        }
    }
    Ok(
        json!({"differentPixels":different,"differentBounds":bounds,"width":ar.width,"height":ar.height}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Raster {
        Raster {
            width: 3,
            height: 2,
            rgba: vec![20; 24],
        }
    }
    fn whole() -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 3,
            height: 2,
        }
    }

    #[test]
    fn pixel_comparison_detects_one_changed_pixel_and_its_position() {
        let original = image();
        let mut changed = image();
        assert_eq!(
            compare(&original, whole(), &changed, whole()).unwrap()["differentPixels"],
            0
        );
        changed.rgba[4] ^= 1;
        let diff = compare(&original, whole(), &changed, whole()).unwrap();
        assert_eq!(diff["differentPixels"], 1);
        assert_eq!(diff["differentBounds"], json!([1, 0, 1, 0]));
    }

    #[test]
    fn equal_caption_crop_does_not_imply_equal_full_image() {
        let original = image();
        let mut changed = image();
        changed.rgba[0] ^= 1;
        let caption = Rect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };
        assert_eq!(
            compare(&original, caption, &changed, caption).unwrap()["differentPixels"],
            0
        );
        assert_eq!(
            compare(&original, whole(), &changed, whole()).unwrap()["differentPixels"],
            1
        );
    }

    #[test]
    fn comparison_rejects_empty_outside_and_mismatched_crops() {
        for rect in [
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 2,
            },
            Rect {
                x: 3,
                y: 0,
                width: 1,
                height: 1,
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        ] {
            assert!(compare(&image(), rect, &image(), whole()).is_err());
        }
    }

    #[test]
    fn fractional_crop_uses_integer_coordinates_inside_native_hit_interval() {
        let crop = Rect::read(&json!({"x":1.25,"y":0.5,"width":2.5,"height":1.5})).unwrap();
        assert_eq!(json!(crop), json!({"x":2,"y":1,"width":2,"height":1}));
        assert!(Rect::read(&json!({"x":-1,"y":0,"width":1,"height":1})).is_err());
    }
}
