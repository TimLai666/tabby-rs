//! Windows/Wry frame regression driver. Run with --help for isolated paths.

#[cfg(windows)]
#[path = "windows_frame_native/caption.rs"]
mod caption;
#[cfg(windows)]
#[path = "windows_frame_native/checks.rs"]
mod checks;
#[cfg(windows)]
#[path = "windows_frame_native/input.rs"]
mod input;
#[cfg(windows)]
#[path = "windows_frame_native/native.rs"]
mod native;
#[cfg(windows)]
#[path = "windows_frame_native/reference.rs"]
mod reference;
#[cfg(windows)]
#[path = "windows_frame_native/renderer.rs"]
mod renderer;

#[cfg(windows)]
mod desktop {
    #[derive(Clone)]
    pub struct VibrancyOptions {
        pub enabled: bool,
        pub effect: Option<String>,
    }
}
#[cfg(windows)]
mod error {
    #[derive(Debug)]
    pub enum AppError {
        InvalidArgument(String),
        Io(String),
        Unsupported(String),
    }
}
#[cfg(windows)]
#[path = "../src/platform/mod.rs"]
mod platform;

#[cfg(windows)]
#[derive(serde::Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: serde_json::Value,
}
#[cfg(windows)]
pub fn check(name: &str, ok: bool, detail: serde_json::Value) -> Check {
    Check {
        name: name.into(),
        ok,
        detail,
    }
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Windows/Wry native frame checks\nUsage: windows_frame_native --output <NEW-DIRECTORY> --data-dir <NEW-DIRECTORY> --electron <PINNED-ELECTRON.EXE>\nOutput and data paths must be fresh. The caller removes the owned browser data directory after process exit.\nWrites result.json and owned native PNGs, including Electron 38.8.6 reference windows. Returns 1 on failed checks, 2 for invalid arguments or a non-Windows host.\nThese are controlled native fixtures, not full Tabby or Windows 11 Snap acceptance.");
        return;
    }
    #[cfg(windows)]
    windows_main(args);
    #[cfg(not(windows))]
    {
        eprintln!("windows_frame_native requires Windows; no native checks ran");
        std::process::exit(2);
    }
}

#[cfg(windows)]
fn windows_main(args: Vec<String>) {
    use serde_json::json;
    use std::{fs, path::PathBuf, sync::mpsc::sync_channel, time::Duration};
    let parse = || -> Result<(PathBuf, PathBuf, PathBuf), String> {
        if args.len() != 6 {
            return Err("expected --output, --data-dir and --electron; use --help".into());
        }
        let mut output = None;
        let mut data = None;
        let mut electron = None;
        for pair in args.chunks_exact(2) {
            match pair[0].as_str() {
                "--output" if output.is_none() => output = Some(PathBuf::from(&pair[1])),
                "--data-dir" if data.is_none() => data = Some(PathBuf::from(&pair[1])),
                "--electron" if electron.is_none() => electron = Some(PathBuf::from(&pair[1])),
                _ => return Err("unknown or duplicate option; use --help".into()),
            }
        }
        let output = output.ok_or("missing --output")?;
        let data = data.ok_or("missing --data-dir")?;
        let electron = electron.ok_or("missing --electron")?;
        if !electron.is_file() {
            return Err("pinned Electron executable is missing".into());
        }
        if output == data || output.starts_with(&data) || data.starts_with(&output) {
            return Err("output and data directories must be separate".into());
        }
        if output.exists() || data.exists() {
            return Err("output or data directory already exists".into());
        }
        fs::create_dir(&output).map_err(|e| e.to_string())?;
        fs::create_dir(&data).map_err(|e| e.to_string())?;
        Ok((output, data, electron))
    };
    let (output, data, electron) = match parse() {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let watchdog_output = output.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(180));
        if let Ok(file) = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(watchdog_output.join("result.json"))
        {
            let _ = serde_json::to_writer_pretty(
                file,
                &json!({"kind":"tabby-rs-wry-frame-probe", "ok":false,"errors":["native driver watchdog timed out"]}),
            );
        }
        std::process::exit(1);
    });
    let mut context = tauri::generate_context!();
    context.config_mut().identifier =
        format!("io.tabbyrs.tests.windows-frame.{}", std::process::id());
    context.config_mut().app.windows.clear();
    let reference_checks = reference::run(&electron, &data, &output);
    let (finished, completion) = sync_channel(1);
    let failed_output = output.clone();
    let setup_failed_output = output.clone();
    let built = tauri::Builder::default().setup(move |app| {
        let setup_finished = finished.clone();
        let startup = (|| -> Result<(), Box<dyn std::error::Error>> {
        let first = tauri::WebviewWindowBuilder::new(app, "frameMain", tauri::WebviewUrl::External("about:blank".parse()?))
            .title("Tabby RS controlled Wry frame probe")
            .inner_size(800.0, 580.0).position(80.0, 60.0)
            .decorations(false).transparent(true).visible(true).always_on_top(true)
            .incognito(true).data_directory(data.clone())
            .on_navigation(|url| url.as_str() == "about:blank")
            .build()?;
        let input_data = data.join("caption-input");
        let other = tauri::WebviewWindowBuilder::new(app, "frameSecondary", tauri::WebviewUrl::External("about:blank".parse()?))
            .title("Tabby RS isolated secondary frame probe")
            .inner_size(640.0, 480.0).visible(false).incognito(true).data_directory(data)
            .build()?;
        let app_handle = app.handle().clone();
        std::thread::spawn(move || {
            let ready = renderer::wait(&first);
            let mut results = reference_checks;
            results.push(check("wry-renderer-ready", ready.is_ok(), json!(ready)));
            if ready.is_ok() {
                results.extend(input::run(&first, &other, &input_data, &output));
                results.extend(checks::run(&first, &other, &output));
            } else {
                let captured = native::capture(&first, &output.join("wry-readiness-failure.png"))
                    .map(|image| json!({"width": image.width, "height": image.height, "bodyPixel":image.pixel(20,100)}));
                results.push(check("wry-readiness-failure-capture", captured.is_ok(), json!(captured)));
            }
            for window in [&first, &other] {
                let closed = native::destroy(window);
                results.push(check(&format!("destroy-{}", window.label()), closed.is_ok(), json!(closed)));
            }
            let ok = !results.is_empty() && results.iter().all(|check| check.ok);
            let result = json!({"kind":"tabby-rs-wry-frame-probe", "ok":ok, "sourceRevision":std::env::var("GITHUB_SHA").ok(), "processId":std::process::id(), "productionAppVerified":false,"win11SnapVerified":false,"checks":results,"errors":[]});
            let saved = fs::OpenOptions::new().write(true).create_new(true).open(output.join("result.json"))
                .and_then(|file| serde_json::to_writer_pretty(file, &result).map_err(std::io::Error::other));
            let code = if ok && saved.is_ok() { 0 } else { 1 };
            eprintln!("windows_frame_native: checks={} passed={} result_saved={}", result["checks"].as_array().map_or(0, Vec::len), ok, saved.is_ok());
            if let Err(error) = saved { eprintln!("result write failed: {error}"); }
            let _ = finished.send(code);
            app_handle.exit(code);
        });
        Ok(())
        })();
        if let Err(error) = startup {
            let saved = fs::OpenOptions::new().write(true).create_new(true)
                .open(setup_failed_output.join("result.json"))
                .and_then(|file| serde_json::to_writer_pretty(file,
                    &json!({"kind":"tabby-rs-wry-frame-probe","ok":false,"errors":[format!("native window setup failed: {error}")]}))
                    .map_err(std::io::Error::other));
            eprintln!("Native window setup failed: {error}; result_saved={}", saved.is_ok());
            let _ = setup_finished.send(1);
            app.handle().exit(1);
        }
        Ok(())
    }).build(context);
    match built {
        Ok(app) => {
            app.run_return(|_, event| {
                // Keep the event loop alive until the worker saves its cleanup checks.
                if let tauri::RunEvent::ExitRequested {
                    api, code: None, ..
                } = event
                {
                    api.prevent_exit();
                }
            });
        }
        Err(error) => {
            if let Ok(file) = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(failed_output.join("result.json"))
            {
                let _ = serde_json::to_writer_pretty(
                    file,
                    &json!({"kind":"tabby-rs-wry-frame-probe","ok":false,"errors":[error.to_string()]}),
                );
            }
            eprintln!("Windows/Wry startup failed: {error}");
            std::process::exit(1);
        }
    }
    std::process::exit(
        completion
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or(1),
    );
}
