use std::{env, fs, path::Path};

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0_u8;

    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }

        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(format!("invalid Base64 byte: {byte}")),
        };

        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((buffer >> bits) & 0xff) as u8);
            buffer = if bits == 0 {
                0
            } else {
                buffer & ((1_u32 << bits) - 1)
            };
        }
    }

    Ok(output)
}

fn materialize_icon(manifest_dir: &Path, file_name: &str) -> Result<(), String> {
    let encoded_path = manifest_dir.join("icons").join(format!("{file_name}.b64"));
    let output_path = manifest_dir.join("icons").join(file_name);
    let encoded = fs::read_to_string(&encoded_path)
        .map_err(|error| format!("failed to read {}: {error}", encoded_path.display()))?;
    let decoded = decode_base64(&encoded)?;

    if fs::read(&output_path).ok().as_deref() != Some(decoded.as_slice()) {
        fs::write(&output_path, decoded)
            .map_err(|error| format!("failed to write {}: {error}", output_path.display()))?;
    }

    println!("cargo:rerun-if-changed={}", encoded_path.display());
    Ok(())
}

fn main() {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("CARGO_MANIFEST_DIR is not set");

    materialize_icon(&manifest_dir, "icon.png").expect("failed to materialize Tauri PNG icon");
    materialize_icon(&manifest_dir, "icon.ico").expect("failed to materialize Tauri ICO icon");

    let windows_manifest = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = env::var_os("OUT_DIR")
            .map(std::path::PathBuf::from)
            .expect("OUT_DIR is not set");
        let manifest_path = out_dir.join("tabby-rs-test.manifest");
        fs::write(&manifest_path, include_str!("windows-app-manifest.xml"))
            .expect("failed to write the Windows test manifest");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        Some(manifest_path)
    } else {
        None
    };

    let windows_attributes = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows_attributes))
        .expect("failed to run Tauri build script");

    if let Some(manifest_path) = windows_manifest {
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    }
}
