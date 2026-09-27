//! WinSCP integration: locating the installed copy and opening a session.
//!
//! `detect_path` reads the path from the default icon value the WinSCP installer
//! writes to `HKEY_CLASSES_ROOT\WinSCP.Url\DefaultIcon`, and never writes to the
//! registry itself. The submodules take it from there: `key` converts a private
//! key to the format WinSCP accepts, `uri` builds the session URL, and `launch`
//! starts WinSCP and owns the lifetime of the temporary key files.

pub(crate) mod key;
pub(crate) mod launch;
pub(crate) mod prepare;
pub(crate) mod uri;

use std::mem::size_of;

pub fn detect_path() -> Option<String> {
    read_default_icon().as_deref().and_then(parse_default_icon)
}

/// Extracts the executable path from a `DefaultIcon` value such as
/// `"C:\Program Files\WinSCP\WinSCP.exe",0` or `C:\WinSCP\WinSCP.exe,-1`.
fn parse_default_icon(value: &str) -> Option<String> {
    if value.is_empty() || value.contains('\0') {
        return None;
    }

    let path = match value.strip_prefix('"') {
        Some(quoted) => {
            let (path, suffix) = quoted.split_once('"')?;
            if !suffix.is_empty() && !is_icon_index(suffix.strip_prefix(',')?) {
                return None;
            }
            path
        }
        None if value.contains('"') => return None,
        None => match value.rsplit_once(',') {
            Some((path, index)) if is_icon_index(index) => path,
            Some(_) => return None,
            None => value,
        },
    };

    (!path.is_empty()).then(|| path.to_owned())
}

fn is_icon_index(index: &str) -> bool {
    let digits = match index.as_bytes().first() {
        Some(b'-') | Some(b'+') => &index[1..],
        _ => index,
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Turns the UTF-16 payload written by the registry API into a `String`.
///
/// `byte_len` is the byte count the API reported, which always covers the
/// trailing UTF-16 terminator. An odd or oversized count is rejected instead
/// of being truncated.
fn decode_default_icon(buffer: &[u16], byte_len: u32) -> Option<String> {
    let unit = size_of::<u16>();
    if byte_len % unit as u32 != 0 || byte_len as usize > buffer.len() * unit {
        return None;
    }
    let (terminator, value) = buffer.get(..byte_len as usize / unit)?.split_last()?;
    if *terminator != 0 || value.is_empty() || value.contains(&0) {
        return None;
    }
    String::from_utf16(value).ok()
}

#[cfg(windows)]
fn read_default_icon() -> Option<String> {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CLASSES_ROOT, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RRF_ZEROONFAILURE,
    };

    const SUBKEY: &str = r"WinSCP.Url\DefaultIcon";
    const MAX_VALUE_BYTES: usize = 64 * 1024;

    let subkey: Vec<u16> = SUBKEY.encode_utf16().chain(Some(0)).collect();
    // `RRF_NOEXPAND` is left off so the API expands a REG_EXPAND_SZ value.
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_ZEROONFAILURE;
    // `Vec<u16>` keeps the buffer aligned for the UTF-16 data the API writes.
    let mut buffer = vec![0u16; MAX_VALUE_BYTES / size_of::<u16>()];
    let mut byte_len = MAX_VALUE_BYTES as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            subkey.as_ptr(),
            std::ptr::null(),
            flags,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast::<c_void>(),
            &mut byte_len,
        )
    };

    if status == ERROR_SUCCESS {
        decode_default_icon(&buffer, byte_len)
    } else {
        None
    }
}

#[cfg(not(windows))]
fn read_default_icon() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::{decode_default_icon, detect_path, parse_default_icon};

    fn utf16_with_terminator(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    #[test]
    fn parses_quoted_default_icon_with_index() {
        assert_eq!(
            parse_default_icon(r#""C:\Program Files\WinSCP\WinSCP.exe",0"#),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn parses_quoted_default_icon_without_index() {
        assert_eq!(
            parse_default_icon(r#""C:\Program Files\WinSCP\WinSCP.exe""#),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn keeps_commas_and_unicode_inside_quotes() {
        assert_eq!(
            parse_default_icon("\"C:\\工具,資料\\WinSCP.exe\",-1"),
            Some("C:\\工具,資料\\WinSCP.exe".into())
        );
    }

    #[test]
    fn parses_unquoted_default_icon_with_index() {
        assert_eq!(
            parse_default_icon(r"C:\Program Files\WinSCP\WinSCP.exe,0"),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn parses_unquoted_default_icon_with_negative_index() {
        assert_eq!(
            parse_default_icon(r"C:\Program Files\WinSCP\WinSCP.exe,-1"),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn parses_unquoted_default_icon_without_index() {
        assert_eq!(
            parse_default_icon(r"C:\Program Files\WinSCP\WinSCP.exe"),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn rejects_empty_and_invalid_default_icons() {
        for value in [
            "",
            "\"",
            "\"\u{0}\"",
            "\"\u{0}\",0",
            r#""C:\Program Files\WinSCP\WinSCP.exe"#,
            r#"C:\Program Files\"WinSCP.exe",0"#,
            r#""C:\Program Files\WinSCP\WinSCP.exe",x"#,
            r#""C:\Program Files\WinSCP\WinSCP.exe","#,
            r#"",0"#,
            ",0",
            ",abc",
        ] {
            assert_eq!(parse_default_icon(value), None, "{value:?}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn detect_path_is_none_off_windows() {
        assert_eq!(detect_path(), None);
    }

    #[test]
    fn decodes_utf16_value_with_trailing_terminator() {
        let buffer = utf16_with_terminator(r"C:\Program Files\WinSCP\WinSCP.exe");
        let byte_len = (buffer.len() * 2) as u32;
        assert_eq!(
            decode_default_icon(&buffer, byte_len),
            Some(r"C:\Program Files\WinSCP\WinSCP.exe".into())
        );
    }

    #[test]
    fn decodes_unicode_utf16_value() {
        let buffer = utf16_with_terminator("C:\\工具,資料\\WinSCP.exe");
        let byte_len = (buffer.len() * 2) as u32;
        assert_eq!(
            decode_default_icon(&buffer, byte_len),
            Some("C:\\工具,資料\\WinSCP.exe".into())
        );
    }

    #[test]
    fn decodes_empty_utf16_value() {
        assert_eq!(decode_default_icon(&[0], 2), None);
        assert_eq!(decode_default_icon(&[], 0), None);
    }

    #[test]
    fn rejects_odd_byte_len() {
        let buffer = utf16_with_terminator("C:\\WinSCP.exe");
        assert_eq!(decode_default_icon(&buffer, 3), None);
    }

    #[test]
    fn rejects_byte_len_beyond_buffer() {
        let buffer = utf16_with_terminator("C:\\WinSCP.exe");
        assert_eq!(decode_default_icon(&buffer, 64 * 1024), None);
    }

    #[test]
    fn rejects_value_without_trailing_terminator() {
        let buffer: Vec<u16> = "C:\\WinSCP.exe".encode_utf16().collect();
        let byte_len = (buffer.len() * 2) as u32;
        assert_eq!(decode_default_icon(&buffer, byte_len), None);
    }

    #[test]
    fn rejects_invalid_utf16() {
        let buffer = vec![0xD800u16, 0x0041, 0x0000];
        assert_eq!(decode_default_icon(&buffer, 6), None);
    }

    #[test]
    fn rejects_interior_terminator() {
        let buffer: Vec<u16> = "C:\\Win\0SCP.exe".encode_utf16().chain(Some(0)).collect();
        let byte_len = (buffer.len() * 2) as u32;
        assert_eq!(decode_default_icon(&buffer, byte_len), None);
    }
}
