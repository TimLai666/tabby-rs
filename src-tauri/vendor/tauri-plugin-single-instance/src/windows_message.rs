// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

pub(crate) const LEGACY_DATA_ID: usize = 1542;
pub(crate) const JSON_DATA_ID: usize = 1543;
pub(crate) const QUERY_MESSAGE: u32 = 0x8000 + 1542;
pub(crate) const JSON_PROTOCOL_REPLY: isize = 0x5442_0001;

#[derive(Clone, Copy, Debug)]
pub(crate) enum WireFormat {
    Legacy,
    Json,
}

// Owned payloads preserve the callback's complete argument array and cwd.
pub(crate) fn encode_payload(
    cwd: &str,
    args: &[String],
    format: WireFormat,
) -> Result<Vec<u8>, String> {
    match format {
        WireFormat::Json => {
            if cwd.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
                return Err(
                    "cannot encode a NUL character in the Windows instance payload".to_string(),
                );
            }
            serde_json::to_vec(&(cwd, args))
                .map_err(|error| format!("failed to encode the Windows instance payload: {error}"))
        }
        WireFormat::Legacy => {
            if cwd.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
                return Err(
                    "cannot encode a NUL character in the Windows instance payload".to_string(),
                );
            }
            if cwd.contains('|') || args.iter().any(|arg| arg.contains('|')) {
                return Err(
                    "the legacy Windows instance channel cannot transmit a '|' character; \
                     restart the application so the running instance receives the launch request"
                        .to_string(),
                );
            }
            let mut fields = Vec::with_capacity(args.len() + 1);
            fields.push(cwd);
            fields.extend(args.iter().map(String::as_str));
            let mut bytes = fields.join("|").into_bytes();
            bytes.push(0);
            Ok(bytes)
        }
    }
}

pub(crate) fn decode_payload(bytes: &[u8], format: WireFormat) -> Option<(Vec<String>, String)> {
    match format {
        WireFormat::Json => {
            let (cwd, args) = serde_json::from_slice::<(String, Vec<String>)>(bytes).ok()?;
            if cwd.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
                return None;
            }
            Some((args, cwd))
        }
        WireFormat::Legacy => {
            let terminator = bytes.iter().position(|byte| *byte == 0)?;
            let text = String::from_utf8_lossy(&bytes[..terminator]);
            let mut fields = text.split('|');
            let cwd = fields.next()?.to_string();
            let args = fields.map(str::to_string).collect();
            Some((args, cwd))
        }
    }
}
