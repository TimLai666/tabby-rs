use serde_json::Value;
use std::{
    sync::mpsc::sync_channel,
    thread,
    time::{Duration, Instant},
};

// ExecuteScript readback also captures script exceptions on Windows.
// The navigation-based readiness signal timed out on the Windows runner.
pub fn wait(window: &tauri::WebviewWindow) -> Result<Value, String> {
    let script = r##"(() => {
        try {
            if (document.readyState === 'complete' && document.body && !window.__tabbyFrameProbePainted) {
                document.documentElement.style.cssText = 'margin:0;width:100%;height:100%;background:#1377d1';
                document.body.style.cssText = 'margin:0;width:100%;height:100%;background:#1377d1';
                document.body.innerHTML = '<div style="position:absolute;right:46px;top:0;width:46px;height:32px;background:repeating-conic-gradient(#e1a42b 0 25%,#6c294c 0 50%) 0/8px 8px"></div>';
                window.__tabbyFrameProbePainted = true;
                requestAnimationFrame(() => requestAnimationFrame(() => { window.__tabbyFrameProbeReady = true; }));
            }
            return {ready: window.__tabbyFrameProbeReady === true, painted: window.__tabbyFrameProbePainted === true,
                readyState: document.readyState, hasBody: !!document.body, href: location.href,
                bodyColor: document.body ? getComputedStyle(document.body).backgroundColor : null,
                markerBounds: document.body?.firstElementChild?.getBoundingClientRect().toJSON() ?? null,
                width: innerWidth, height: innerHeight, devicePixelRatio};
        } catch (error) { return {error: String(error), href: location.href, readyState: document.readyState}; }
    })();"##;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (tx, rx) = sync_channel(1);
        window
            .eval_with_callback(script, move |result| {
                let _ = tx.try_send(result);
            })
            .map_err(|error| format!("renderer evaluation dispatch: {error}"))?;
        let raw = rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| format!("renderer evaluation callback: {error}"))?;
        if raw.len() > 16384 {
            return Err("renderer readback is excessive".into());
        }
        let state: Value = serde_json::from_str(&raw)
            .map_err(|error| format!("renderer readback JSON: {error}: {raw}"))?;
        if !state["error"].is_null() {
            return Err(format!("renderer script failed: {state}"));
        }
        if state["ready"] == true && state["painted"] == true && state["readyState"] == "complete" {
            return Ok(state);
        }
        if Instant::now() >= deadline {
            return Err(format!("renderer readiness timed out: {state}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
