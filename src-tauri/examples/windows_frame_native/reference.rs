use super::{check, native, Check};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

pub fn run(executable: &Path, data: &Path, output: &Path) -> Vec<Check> {
    let mut checks = Vec::new();
    for mode in ["native", "thin", "full"] {
        let result = (|| -> Result<serde_json::Value, String> {
            let ready = output.join(format!("electron-{mode}.json"));
            let close = output.join(format!("electron-{mode}.close"));
            let profile = data.join(format!("electron-{mode}"));
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/windows_frame_native/electron.cjs");
            let child = Command::new(executable)
                .arg(fixture)
                .arg(&profile)
                .arg(&ready)
                .arg(&close)
                .arg(mode)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| e.to_string())?;
            let mut child = OwnedChild(child);
            let deadline = Instant::now() + Duration::from_secs(20);
            while !ready.is_file() {
                if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                    return Err(format!("reference exited before readiness: {status}"));
                }
                if Instant::now() >= deadline {
                    return Err("reference did not signal readiness".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if fs::metadata(&ready).map_err(|e| e.to_string())?.len() > 32768 {
                return Err("reference metadata is excessive".into());
            }
            let manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&ready).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if manifest["processId"].as_u64() != Some(child.0.id() as u64)
                || manifest["electron"] != "38.8.6"
                || manifest["mode"] != mode
            {
                return Err("reference process/version/mode differs from the owned child".into());
            }
            let handle = manifest["hwnd"]
                .as_str()
                .ok_or("reference has no HWND")?
                .parse::<usize>()
                .map_err(|e| e.to_string())?;
            let image = native::capture_owned(
                handle,
                child.0.id(),
                &output.join(format!("electron-{mode}.png")),
                true,
            )?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(close)
                .map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                    break status;
                }
                if Instant::now() >= deadline {
                    return Err("reference did not exit after the owned close signal".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            if !status.success() {
                return Err(format!("reference failed: {status}"));
            }
            Ok(
                json!({"manifest":manifest,"captureWidth":image.width,"captureHeight":image.height,"ownedChildExited":true}),
            )
        })();
        checks.push(check(
            &format!("pinned-electron-{mode}"),
            result.is_ok(),
            json!(result),
        ));
    }
    checks
}
