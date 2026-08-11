pub const MAX_SERIAL_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_HEX_LINE_BYTES: usize = 64 * 1024;

pub fn parse_hex_line(input: &str) -> Result<Vec<u8>, String> {
    if input.len() > MAX_HEX_LINE_BYTES {
        return Err(format!("hex input line exceeds {MAX_HEX_LINE_BYTES} bytes"));
    }
    let mut output = Vec::new();
    for (index, token) in input.split_whitespace().enumerate() {
        let token = token.strip_prefix("0x").unwrap_or(token);
        if token.len() != 2 || token.chars().any(|c| !c.is_ascii_hexdigit()) {
            return Err(format!("invalid hex token at index {index}: {token}"));
        }
        output.push(
            u8::from_str_radix(token, 16)
                .map_err(|_| format!("invalid hex token at index {index}: {token}"))?,
        );
    }
    Ok(output)
}

pub fn hexdump(data: &[u8], offset: u64) -> String {
    let mut output = String::new();
    for (line, chunk) in data.chunks(16).enumerate() {
        let line_offset = offset + line as u64 * 16;
        let hex = chunk
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let ascii = chunk
            .iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    *byte as char
                } else {
                    '.'
                }
            })
            .collect::<String>();
        output.push_str(&format!("{line_offset:08x}  {hex:<47}  |{ascii:<16}|\n"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_without_sending_partial_input() {
        assert_eq!(parse_hex_line("00 ff 0x1a"), Ok(vec![0, 255, 0x1a]));
        assert!(parse_hex_line("00 gg").is_err());
        assert!(parse_hex_line("123").is_err());
        assert!(parse_hex_line(&"00 ".repeat(MAX_HEX_LINE_BYTES)).is_err());
    }

    #[test]
    fn hexdump_keeps_offset_across_chunks() {
        let rendered = hexdump(&(0..20).collect::<Vec<_>>(), 16);
        assert!(rendered.starts_with("00000010"));
        assert!(rendered.contains("00000020"));
        assert!(rendered.contains("|................|"));
    }
}
