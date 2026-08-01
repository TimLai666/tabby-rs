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

    tauri_build::build()
}
