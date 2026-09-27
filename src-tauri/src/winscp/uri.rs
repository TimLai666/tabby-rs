//! Builds the `scp://` session URL WinSCP is opened with.
//!
//! The URL follows `tabby-ssh/src/services/ssh.service.ts` and the syntax
//! documented at https://winscp.net/eng/docs/session_url. Every credential and
//! tunnel value is percent-encoded the way `encodeURIComponent` does, so a
//! password or a host name can never add an `@`, a `;` or a `/` of its own.

use std::path::Path;

use crate::error::AppError;

/// A connection as the front end hands it over.
///
/// This type carries a password, so it deliberately has no `Debug`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionOptions {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    #[serde(default)]
    pub private_keys: Vec<super::key::KeyInput>,
}

/// Builds the session URL for a target and an optional jump host.
///
/// `jump_key` is the converted jump host key file together with its passphrase;
/// it is only added when a jump host is given. Nothing is launched here.
pub(crate) fn connection_uri(
    target: &ConnectionOptions,
    jump: Option<&ConnectionOptions>,
    jump_key: Option<(&Path, &str)>,
) -> Result<String, AppError> {
    validate("target", &target.host, target.port)?;

    let mut uri = format!("scp://{}", encode_component(&target.username));
    if let Some(password) = present(&target.password) {
        uri.push(':');
        uri.push_str(&encode_component(password));
    }

    if let Some(jump) = jump {
        validate("jump", &jump.host, jump.port)?;
        uri.push_str(";x-tunnel=1");
        uri.push_str(";x-tunnelhostname=");
        uri.push_str(&encode_component(unbracket(&jump.host)));
        uri.push_str(";x-tunnelportnumber=");
        uri.push_str(&jump.port.to_string());
        uri.push_str(";x-tunnelusername=");
        uri.push_str(&encode_component(&jump.username));
        if let Some(password) = present(&jump.password) {
            uri.push_str(";x-tunnelpasswordplain=");
            uri.push_str(&encode_component(password));
        }
        if let Some((path, passphrase)) = jump_key {
            uri.push_str(";x-tunnelpublickeyfile=");
            uri.push_str(&encode_component(&path.to_string_lossy()));
            uri.push_str(";x-tunnelpassphraseplain=");
            uri.push_str(&encode_component(passphrase));
        }
    }

    uri.push('@');
    uri.push_str(&authority_host(&target.host));
    uri.push(':');
    uri.push_str(&target.port.to_string());
    uri.push('/');
    Ok(uri)
}

/// Rejects what cannot appear in a URL authority. The messages name the field
/// but never its value, so a rejected host is not echoed back.
fn validate(label: &str, host: &str, port: u16) -> Result<(), AppError> {
    if host.is_empty() {
        return Err(AppError::InvalidArgument(format!(
            "{label} host must not be empty"
        )));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AppError::InvalidArgument(format!(
            "{label} host must not contain control characters or whitespace"
        )));
    }
    if port == 0 {
        return Err(AppError::InvalidArgument(format!(
            "{label} port must not be zero"
        )));
    }
    Ok(())
}

/// An absent or empty password is left out, the same way the reference
/// implementation omits a falsy one.
fn present(password: &Option<String>) -> Option<&str> {
    password.as_deref().filter(|value| !value.is_empty())
}

/// Formats the host for the target authority. An IPv6 literal keeps its colons
/// and is bracketed; anything else is one encoded component, which keeps a
/// stray `@`, `;` or `/` from ending the authority.
fn authority_host(host: &str) -> String {
    if !host.contains(':') {
        return encode_component(host);
    }
    let literal = unbracket(host);
    let mut encoded = String::with_capacity(literal.len() + 2);
    encoded.push('[');
    for byte in literal.bytes() {
        // WinSCP's ParseUrl (source/core/SessionData.cpp) takes everything
        // between the brackets as `HostName` verbatim, while the unbracketed
        // branch runs it through `DecodeUrlChars`. A raw `%` must therefore
        // survive here, or a scoped address such as `fe80::1%12` would reach
        // WinSCP as `fe80::1%2512`. Delimiters still get encoded because
        // `LastDelimiter(L"@")` runs over the whole authority first.
        if byte == b':' || byte == b'%' || is_safe(byte) {
            encoded.push(char::from(byte));
        } else {
            push_escape(&mut encoded, byte);
        }
    }
    encoded.push(']');
    encoded
}

/// Drops one pair of brackets from an IPv6 literal that already carries them.
/// A host without a colon is not IPv6, so its brackets are part of the name and
/// stay put.
fn unbracket(host: &str) -> &str {
    if !host.contains(':') {
        return host;
    }
    host.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host)
}

/// Percent-encodes a value the way `encodeURIComponent` does, over UTF-8 bytes
/// and with upper case hex digits. This is not form encoding: a space becomes
/// `%20` and never a `+`.
fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if is_safe(byte) {
            encoded.push(char::from(byte));
        } else {
            push_escape(&mut encoded, byte);
        }
    }
    encoded
}

fn push_escape(encoded: &mut String, byte: u8) {
    encoded.push('%');
    encoded.push(char::from(HEX[usize::from(byte >> 4)]));
    encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// The characters `encodeURIComponent` leaves alone, so dots stay readable and
/// `!~*'()` stay literal.
fn is_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
        )
}

#[cfg(test)]
mod tests {
    use super::{connection_uri, ConnectionOptions};
    use crate::error::AppError;
    use std::path::Path;

    fn options(host: &str, port: u16, username: &str, password: Option<&str>) -> ConnectionOptions {
        ConnectionOptions {
            host: host.into(),
            port,
            username: username.into(),
            password: password.map(str::to_owned),
            private_keys: Vec::new(),
        }
    }

    #[test]
    fn builds_plain_session_uri() {
        let uri = connection_uri(&options("example.com", 22, "alice", None), None, None).unwrap();
        assert_eq!(uri, "scp://alice@example.com:22/");
    }

    #[test]
    fn builds_uri_with_password() {
        let uri = connection_uri(
            &options("example.com", 2222, "alice", Some("s3cret")),
            None,
            None,
        )
        .unwrap();
        assert_eq!(uri, "scp://alice:s3cret@example.com:2222/");
    }

    #[test]
    fn omits_missing_and_empty_passwords() {
        for password in [None, Some("")] {
            let uri =
                connection_uri(&options("example.com", 22, "alice", password), None, None).unwrap();
            assert_eq!(uri, "scp://alice@example.com:22/");
        }
    }

    #[test]
    fn keeps_encode_uri_component_safe_characters() {
        let uri = connection_uri(
            &options("example.com", 22, "alice", Some("a-b_c.d!~*'()")),
            None,
            None,
        )
        .unwrap();
        assert_eq!(uri, "scp://alice:a-b_c.d!~*'()@example.com:22/");
    }

    #[test]
    fn encodes_unicode_credentials() {
        let uri = connection_uri(
            &options("example.com", 22, "alïce", Some("päss🔑")),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            uri,
            "scp://al%C3%AFce:p%C3%A4ss%F0%9F%94%91@example.com:22/"
        );
    }

    #[test]
    fn encodes_credential_delimiters() {
        let uri = connection_uri(
            &options("example.com", 22, "a@b;c/d", Some("p@ss;/word")),
            None,
            None,
        )
        .unwrap();
        assert_eq!(uri, "scp://a%40b%3Bc%2Fd:p%40ss%3B%2Fword@example.com:22/");
        assert_eq!(uri.matches('@').count(), 1);
    }

    #[test]
    fn brackets_ipv6_target_host() {
        for (host, expected) in [("::1", "[::1]"), ("[2001:db8::1]", "[2001:db8::1]")] {
            let uri = connection_uri(&options(host, 22, "alice", None), None, None).unwrap();
            assert_eq!(uri, format!("scp://alice@{expected}:22/"), "{host}");
        }
    }

    #[test]
    fn keeps_a_raw_percent_in_a_bracketed_ipv6_target_host() {
        for host in ["fe80::1%12", "[fe80::1%12]"] {
            let uri = connection_uri(&options(host, 22, "alice", None), None, None).unwrap();
            assert_eq!(uri, "scp://alice@[fe80::1%12]:22/", "{host}");
        }

        let jump = options("fe80::1%12", 22, "bob", None);
        let uri = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            None,
        )
        .unwrap();
        assert!(uri.contains(";x-tunnelhostname=fe80%3A%3A1%2512;"), "{uri}");
    }

    #[test]
    fn encodes_target_host_as_a_component() {
        let uri = connection_uri(&options("evil@x;y/z", 22, "alice", None), None, None).unwrap();
        assert_eq!(uri, "scp://alice@evil%40x%3By%2Fz:22/");
        assert_eq!(uri.matches('@').count(), 1);
        assert_eq!(uri.matches(';').count(), 0);
    }

    #[test]
    fn builds_jump_tunnel_uri() {
        let jump = options("jump.example.net", 2222, "bob", Some("j0ump"));
        let uri = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            None,
        )
        .unwrap();
        assert_eq!(
            uri,
            "scp://alice;x-tunnel=1;x-tunnelhostname=jump.example.net;x-tunnelportnumber=2222\
             ;x-tunnelusername=bob;x-tunnelpasswordplain=j0ump@example.com:22/"
        );
    }

    #[test]
    fn encodes_jump_hostname_path_and_passphrase() {
        let jump = options("[::1]", 22, "b/b", None);
        let key = (Path::new("C:\\Temp\\鍵 1.ppk"), "p@ss;phrase");
        let uri = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            Some(key),
        )
        .unwrap();
        assert_eq!(
            uri,
            "scp://alice;x-tunnel=1;x-tunnelhostname=%3A%3A1;x-tunnelportnumber=22\
             ;x-tunnelusername=b%2Fb;x-tunnelpublickeyfile=C%3A%5CTemp%5C%E9%8D%B5%201.ppk\
             ;x-tunnelpassphraseplain=p%40ss%3Bphrase@example.com:22/"
        );
        assert_eq!(uri.matches(';').count(), 6);
    }

    #[test]
    fn keeps_brackets_on_a_jump_host_that_is_not_ipv6() {
        let jump = options("[literal]", 22, "bob", None);
        let uri = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            None,
        )
        .unwrap();
        assert_eq!(
            uri,
            "scp://alice;x-tunnel=1;x-tunnelhostname=%5Bliteral%5D;x-tunnelportnumber=22\
             ;x-tunnelusername=bob@example.com:22/"
        );
    }

    #[test]
    fn ignores_jump_key_without_a_jump_host() {
        let key = (Path::new("C:\\Temp\\id.ppk"), "phrase");
        let uri =
            connection_uri(&options("example.com", 22, "alice", None), None, Some(key)).unwrap();
        assert_eq!(uri, "scp://alice@example.com:22/");
    }

    #[test]
    fn reads_camel_case_input_from_the_front_end() {
        let parsed: ConnectionOptions =
            serde_json::from_str(r#"{"host":"example.com","port":22,"username":"alice"}"#).unwrap();
        assert_eq!(parsed.host, "example.com");
        assert_eq!(parsed.port, 22);
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.password, None);
        assert!(parsed.private_keys.is_empty());

        let parsed: ConnectionOptions = serde_json::from_str(
            r#"{"host":"h","port":22,"username":"u","password":"p","privateKeys":[{"content":"k"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.password.as_deref(), Some("p"));
        assert_eq!(parsed.private_keys.len(), 1);
        assert_eq!(parsed.private_keys[0].content, "k");
    }

    #[test]
    fn rejects_zero_ports() {
        let error =
            connection_uri(&options("example.com", 0, "alice", None), None, None).unwrap_err();
        assert!(matches!(&error, AppError::InvalidArgument(_)), "{error:?}");

        let jump = options("jump.example.net", 0, "bob", None);
        let error = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            None,
        )
        .unwrap_err();
        assert!(matches!(&error, AppError::InvalidArgument(_)), "{error:?}");
    }

    #[test]
    fn rejects_empty_hosts() {
        let error = connection_uri(&options("", 22, "alice", None), None, None).unwrap_err();
        assert!(matches!(&error, AppError::InvalidArgument(_)), "{error:?}");

        let jump = options("", 22, "bob", None);
        let error = connection_uri(
            &options("example.com", 22, "alice", None),
            Some(&jump),
            None,
        )
        .unwrap_err();
        assert!(matches!(&error, AppError::InvalidArgument(_)), "{error:?}");
    }

    #[test]
    fn rejects_control_and_whitespace_in_hosts() {
        for host in [
            "bad host",
            "bad\thost",
            "bad\nhost",
            "bad\r\nhost",
            "bad\0host",
        ] {
            let error = connection_uri(&options(host, 22, "alice", Some("hunter2")), None, None)
                .unwrap_err();
            let rendered = format!("{error}");
            assert!(matches!(&error, AppError::InvalidArgument(_)), "{host:?}");
            assert!(!rendered.contains("hunter2"), "{host:?}: {rendered}");
            assert!(!rendered.contains(host), "{host:?}: {rendered}");
        }
    }
}
