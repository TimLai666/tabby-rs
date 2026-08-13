use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct RedactionContext {
    pub known_secrets: Vec<String>,
    pub home_dir: Option<String>,
    pub usernames: Vec<String>,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactedText {
    pub text: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Redactor {
    context: RedactionContext,
    host_placeholders: BTreeMap<String, String>,
    username_placeholders: BTreeMap<String, String>,
}

impl Redactor {
    pub fn from_environment() -> Self {
        Self::new(RedactionContext {
            home_dir: std::env::var("HOME")
                .ok()
                .or_else(|| std::env::var("USERPROFILE").ok()),
            usernames: std::env::var("USER")
                .ok()
                .into_iter()
                .chain(std::env::var("USERNAME").ok())
                .collect(),
            ..RedactionContext::default()
        })
    }

    pub fn from_storage_directory(directory: &std::path::Path) -> Self {
        Self::from_storage_directory_with_secrets(directory, &[])
    }

    pub fn from_storage_directory_with_secrets(
        directory: &std::path::Path,
        known_secrets: &[String],
    ) -> Self {
        let mut redactor = Self::from_environment();
        redactor.context.known_secrets.extend(
            known_secrets
                .iter()
                .filter(|value| !value.is_empty())
                .cloned(),
        );
        let Some(data_dir) = directory.parent() else {
            return redactor;
        };
        if let Ok(bytes) = std::fs::read(data_dir.join("config.yaml")) {
            if bytes.len() <= 16 * 1024 * 1024 {
                if let Ok(value) = serde_yaml::from_slice::<serde_yaml::Value>(&bytes) {
                    collect_config_identifiers(&value, &mut redactor.context);
                }
            }
        }
        if let Ok(contents) = std::fs::read_to_string(data_dir.join("known_hosts")) {
            for line in contents.lines() {
                let Some(hosts) = line.split_whitespace().next() else {
                    continue;
                };
                if hosts.starts_with('|') {
                    continue;
                }
                redactor.context.hosts.extend(
                    hosts
                        .split(',')
                        .filter(|host| !host.is_empty())
                        .map(ToOwned::to_owned),
                );
            }
        }
        Self::new(redactor.context)
    }

    pub fn new(context: RedactionContext) -> Self {
        let host_placeholders = indexed_placeholders(&context.hosts, "HOST");
        let username_placeholders = indexed_placeholders(&context.usernames, "USER");
        Self {
            context,
            host_placeholders,
            username_placeholders,
        }
    }

    pub fn redact_text(&self, input: &str) -> RedactedText {
        let mut text = input.to_owned();
        let mut redacted = false;
        if let Some(home) = self.context.home_dir.as_deref() {
            redacted |= replace_all(&mut text, home, "<HOME>");
        }
        for secret in self
            .context
            .known_secrets
            .iter()
            .filter(|value| !value.is_empty())
        {
            redacted |= replace_all(&mut text, secret, "<SECRET>");
        }
        redacted |= redact_private_key_blocks(&mut text);
        redacted |= redact_url_credentials(&mut text);
        redacted |= redact_auth_headers(&mut text);
        redacted |= redact_usernames_in_authorities(&mut text, &self.context.usernames);
        redacted |= redact_hosts_in_authorities(&mut text, &self.context.hosts);
        redacted |= redact_emails(&mut text);
        redacted |= redact_ipv6s(&mut text);
        redacted |= redact_ips(&mut text);
        for (username, placeholder) in replacement_order(&self.username_placeholders) {
            redacted |= replace_all(&mut text, username, placeholder);
        }
        for (host, placeholder) in replacement_order(&self.host_placeholders) {
            redacted |= replace_all(&mut text, host, placeholder);
        }
        RedactedText { text, redacted }
    }

    pub fn redact_json(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                let mut output = serde_json::Map::new();
                for (key, value) in object {
                    if is_sensitive_key(key) {
                        output.insert(key.clone(), serde_json::Value::String("<REDACTED>".into()));
                    } else {
                        output.insert(key.clone(), self.redact_json(value));
                    }
                }
                serde_json::Value::Object(output)
            }
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values.iter().map(|value| self.redact_json(value)).collect(),
            ),
            serde_json::Value::String(value) => {
                serde_json::Value::String(self.redact_text(value).text)
            }
            value => value.clone(),
        }
    }
}

fn collect_config_identifiers(value: &serde_yaml::Value, context: &mut RedactionContext) {
    match value {
        serde_yaml::Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let key = key.as_str().unwrap_or_default().to_ascii_lowercase();
                if matches!(key.as_str(), "host" | "hostname") {
                    if let Some(value) = value.as_str() {
                        context.hosts.push(value.to_owned());
                    }
                } else if matches!(key.as_str(), "user" | "username") {
                    if let Some(value) = value.as_str() {
                        context.usernames.push(value.to_owned());
                    }
                }
                collect_config_identifiers(value, context);
            }
        }
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                collect_config_identifiers(value, context);
            }
        }
        _ => {}
    }
}

fn indexed_placeholders(values: &[String], label: &str) -> BTreeMap<String, String> {
    let values = values
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter();
    let mut placeholders = BTreeMap::new();
    for (index, value) in values.enumerate() {
        placeholders.insert(value, format!("<{label}:{}>", index + 1));
    }
    placeholders
}

fn replacement_order(placeholders: &BTreeMap<String, String>) -> Vec<(&str, &str)> {
    let mut values = placeholders
        .iter()
        .map(|(value, placeholder)| (value.as_str(), placeholder.as_str()))
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .0
            .len()
            .cmp(&left.0.len())
            .then_with(|| left.0.cmp(right.0))
    });
    values
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passphrase",
        "token",
        "secret",
        "privatekey",
        "authorization",
        "cookie",
    ]
    .iter()
    .any(|part| key.contains(part))
}

fn replace_all(text: &mut String, needle: &str, replacement: &str) -> bool {
    if !text.contains(needle) {
        return false;
    }
    *text = text.replace(needle, replacement);
    true
}

fn redact_private_key_blocks(text: &mut String) -> bool {
    let mut output = String::with_capacity(text.len());
    let mut inside = false;
    let mut redacted = false;
    for line in text.lines() {
        if line.contains("-----BEGIN ") && line.contains(" PRIVATE KEY-----") {
            inside = true;
            redacted = true;
            output.push_str("<PRIVATE_KEY>\n");
        } else if inside {
            if line.contains("-----END ") && line.contains(" PRIVATE KEY-----") {
                inside = false;
            }
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    if !redacted {
        return false;
    }
    if !text.ends_with('\n') && output.ends_with('\n') {
        output.pop();
    }
    *text = output;
    true
}

fn redact_url_credentials(text: &mut String) -> bool {
    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let Some(scheme_end) = token.find("://") else {
            output.push_str(token);
            continue;
        };
        let authority_start = scheme_end + 3;
        let Some(at) = token[authority_start..].find('@') else {
            output.push_str(token);
            continue;
        };
        let at = authority_start + at;
        let credentials = &token[authority_start..at];
        if credentials.contains(':') {
            output.push_str(&token[..authority_start]);
            output.push_str("<USER>:<SECRET>@");
            output.push_str(&token[at + 1..]);
            changed = true;
        } else {
            output.push_str(token);
        }
    }
    *text = output;
    changed
}

fn redact_auth_headers(text: &mut String) -> bool {
    let mut output = String::with_capacity(text.len());
    let mut previous_was_auth = false;
    let mut changed = false;
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_matches(char::is_whitespace);
        if previous_was_auth && !trimmed.is_empty() {
            let prefix_len = token.len() - token.trim_start_matches(char::is_whitespace).len();
            let suffix_len = token.len() - token.trim_end_matches(char::is_whitespace).len();
            output.push_str(&token[..prefix_len]);
            output.push_str("<SECRET>");
            output.push_str(&token[token.len() - suffix_len..]);
            previous_was_auth = false;
            changed = true;
            continue;
        }
        output.push_str(token);
        previous_was_auth = matches!(trimmed.to_ascii_lowercase().as_str(), "bearer" | "basic");
    }
    if changed {
        *text = output;
    }
    changed
}

fn redact_emails(text: &mut String) -> bool {
    let mut changed = false;
    let tokens = text.split_inclusive(char::is_whitespace).map(|token| {
        if !token.contains("://")
            && token.contains('@')
            && token.split('@').count() == 2
            && !token.contains("<HOST>")
            && !token
                .split('@')
                .next_back()
                .is_some_and(|value| value.contains(':'))
            && !token
                .split('@')
                .next_back()
                .map(|value| value.trim_matches(char::is_whitespace))
                .is_some_and(is_ipv4)
        {
            changed = true;
            let suffix_len = token
                .chars()
                .rev()
                .take_while(|character| character.is_whitespace())
                .count();
            format!("<EMAIL>{}", &token[token.len() - suffix_len..])
        } else {
            token.to_owned()
        }
    });
    let output = tokens.collect::<String>();
    if changed {
        *text = output;
    }
    changed
}

fn redact_usernames_in_authorities(text: &mut String, known_usernames: &[String]) -> bool {
    let mut changed = false;
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let Some(at) = token.find('@') else {
            output.push_str(token);
            continue;
        };
        let address = token[at + 1..]
            .trim_matches(|character: char| ",.;()[]{}<> \t\r\n".contains(character));
        if !token.contains("://")
            && !address.contains(':')
            && !is_ipv4(authority_host(address))
            && authority_host(address)
                .parse::<std::net::Ipv6Addr>()
                .is_err()
        {
            output.push_str(token);
            continue;
        }
        let host = authority_host(address);
        if !is_authority_host(host) {
            output.push_str(token);
            continue;
        }
        let user_start = token[..at]
            .char_indices()
            .rev()
            .find_map(|(index, character)| {
                character
                    .is_ascii_whitespace()
                    .then_some(index + character.len_utf8())
                    .or_else(|| ":=,/;([{<".contains(character).then_some(index + 1))
            })
            .unwrap_or(0);
        if user_start == at {
            output.push_str(token);
            continue;
        }
        let username = &token[user_start..at];
        if username.contains('<') || username.contains('>') || username.contains(':') {
            output.push_str(token);
            continue;
        }
        if known_usernames
            .iter()
            .any(|known_username| known_username == username)
        {
            output.push_str(token);
            continue;
        }
        output.push_str(&token[..user_start]);
        output.push_str("<USER>");
        output.push_str(&token[at..]);
        changed = true;
    }
    if changed {
        *text = output;
    }
    changed
}

fn redact_hosts_in_authorities(text: &mut String, known_hosts: &[String]) -> bool {
    let mut changed = false;
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let Some(host_start) = authority_host_start(token) else {
            output.push_str(token);
            continue;
        };
        if !token.contains("://")
            && token[host_start..]
                .split(|character: char| ",;()[]{}<> \t\r\n".contains(character))
                .next()
                .is_some_and(|address| !address.contains(':'))
        {
            output.push_str(token);
            continue;
        }
        let host_end = token[host_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                ":/?#;,(){}<> \t\r\n"
                    .contains(character)
                    .then_some(host_start + offset)
            })
            .unwrap_or(token.len());
        let host = &token[host_start..host_end];
        if !looks_like_hostname(host) || known_hosts.iter().any(|known_host| known_host == host) {
            output.push_str(token);
            continue;
        }
        output.push_str(&token[..host_start]);
        output.push_str("<HOST>");
        output.push_str(&token[host_end..]);
        changed = true;
    }
    if changed {
        *text = output;
    }
    changed
}

fn authority_host_start(token: &str) -> Option<usize> {
    if let Some(at) = token.find('@') {
        return Some(at + 1);
    }
    token.find("://").map(|scheme_end| scheme_end + 3)
}

fn authority_host(address: &str) -> &str {
    let address = address.trim_matches(|character: char| ",.;()[]{}<> \t\r\n".contains(character));
    let address = address.strip_prefix('[').unwrap_or(address);
    address
        .split_once(']')
        .map(|(host, _)| host)
        .unwrap_or_else(|| address.split(':').next().unwrap_or(address))
}

fn is_authority_host(host: &str) -> bool {
    is_ipv4(host) || host.parse::<std::net::Ipv6Addr>().is_ok() || looks_like_hostname(host)
}

fn looks_like_hostname(value: &str) -> bool {
    let value = value.trim_matches(|character: char| ",.;()[]{}<> \t\r\n".contains(character));
    value.contains('.')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '.'
        })
        && value
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}

fn redact_ips(text: &mut String) -> bool {
    let mut changed = false;
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let (token_output, token_changed) = redact_candidates(
            token,
            |character| character.is_ascii_digit() || character == '.',
            is_ipv4,
        );
        changed |= token_changed;
        output.push_str(&token_output);
    }
    if changed {
        *text = output;
    }
    changed
}

fn redact_ipv6s(text: &mut String) -> bool {
    let mut changed = false;
    let mut output = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let (token_output, token_changed) = redact_candidates(
            token,
            |character| character.is_ascii_hexdigit() || character == ':',
            |candidate| candidate.parse::<std::net::Ipv6Addr>().is_ok(),
        );
        changed |= token_changed;
        output.push_str(&token_output);
    }
    if changed {
        *text = output;
    }
    changed
}

fn redact_candidates<F>(
    token: &str,
    is_candidate_character: fn(char) -> bool,
    is_value: F,
) -> (String, bool)
where
    F: Fn(&str) -> bool,
{
    let mut output = String::with_capacity(token.len());
    let mut last = 0;
    let mut start = None;
    let mut changed = false;
    for (index, character) in token
        .char_indices()
        .chain(std::iter::once((token.len(), '\0')))
    {
        if is_candidate_character(character) {
            start.get_or_insert(index);
        } else if let Some(start_index) = start.take() {
            let candidate = &token[start_index..index];
            if is_value(candidate) {
                output.push_str(&token[last..start_index]);
                output.push_str("<IP>");
                last = index;
                changed = true;
            }
        }
    }
    if !changed {
        return (token.to_owned(), false);
    }
    output.push_str(&token[last..]);
    (output, true)
}

fn is_ipv4(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 4 && parts.iter().all(|part| part.parse::<u8>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::{RedactionContext, Redactor};

    fn redactor() -> Redactor {
        Redactor::new(RedactionContext {
            known_secrets: vec!["abc123".into()],
            home_dir: Some("/home/alice".into()),
            usernames: vec!["alice".into()],
            hosts: vec!["server.internal".into()],
        })
    }

    #[test]
    fn redacts_secrets_and_network_identifiers() {
        let output = redactor().redact_text(
            "ssh alice@192.0.2.10 token=abc123 host=server.internal mail=alice@example.com path=/home/alice/.ssh/id_ed25519",
        );
        assert!(output.redacted);
        assert!(!output.text.contains("abc123"));
        assert!(!output.text.contains("192.0.2.10"));
        assert!(!output.text.contains("alice@example.com"));
        assert!(!output.text.contains("server.internal"));
        assert!(!output.text.contains("/home/alice"));
        assert!(output.text.contains("<USER:1>"));
        assert!(output.text.contains("<IP>"));
    }

    #[test]
    fn redacts_unknown_ssh_usernames_before_ip_addresses() {
        let output = Redactor::new(RedactionContext {
            known_secrets: vec!["abc123".into()],
            ..Default::default()
        })
        .redact_text("ssh alice@192.0.2.10 token=abc123");
        assert!(!output.text.contains("alice"));
        assert!(!output.text.contains("192.0.2.10"));
        assert!(output.text.contains("<USER>@<IP>"));
    }

    #[test]
    fn redacts_all_ipv4_and_ipv6_addresses_in_one_token() {
        let output = Redactor::default()
            .redact_text("addresses=192.0.2.1,198.51.100.2 [2001:db8::1],[2001:db8::2]");

        assert_eq!(output.text, "addresses=<IP>,<IP> [<IP>],[<IP>]");
    }

    #[test]
    fn redacts_unknown_hostnames_only_in_authorities() {
        let output = Redactor::default().redact_text(
            "ssh alice@db.internal:22 https://api.example.test/path note=api.example.test",
        );
        assert!(!output.text.contains("alice@db.internal"));
        assert!(output.text.contains("<USER>@<HOST>:22"));
        assert!(output.text.contains("https://<HOST>/path"));
        assert!(output.text.contains("note=api.example.test"));
    }

    #[test]
    fn redacts_private_keys_and_url_credentials() {
        let output = redactor().redact_text(
            "https://alice:abc123@example.com/a\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret material\n-----END OPENSSH PRIVATE KEY-----",
        );
        assert_eq!(
            output.text,
            "https://<USER>:<SECRET>@<HOST>/a\n<PRIVATE_KEY>"
        );
    }

    #[test]
    fn redacts_ipv6_and_authorization_headers() {
        let output = Redactor::default()
            .redact_text("host=[2001:db8::1]:22 Authorization: Bearer abc.def Basic dGVzdA==");
        assert!(output.redacted);
        assert!(!output.text.contains("2001:db8::1"));
        assert!(!output.text.contains("abc.def"));
        assert!(!output.text.contains("dGVzdA=="));
        assert!(output.text.contains("<IP>"));
        assert!(output.text.contains("<SECRET>"));
    }

    #[test]
    fn redacts_sensitive_json_fields_recursively() {
        let value = serde_json::json!({"token": "abc123", "nested": {"host": "server.internal"}, "ok": "alice@example.com"});
        let output = redactor().redact_json(&value);
        assert_eq!(output["token"], "<REDACTED>");
        assert_eq!(output["nested"]["host"], "<HOST:1>");
        assert_eq!(output["ok"], "<EMAIL>");
    }

    #[test]
    fn loads_hosts_and_users_only_as_redaction_context() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.yaml"),
            "profiles:\n  - host: private.example\n    username: bob\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("known_hosts"),
            "known.example ssh-ed25519 AAAA\n",
        )
        .unwrap();
        let redactor = Redactor::from_storage_directory(&temp.path().join("logs"));
        let output = redactor.redact_text("private.example known.example bob");
        assert!(!output.text.contains("private.example"));
        assert!(!output.text.contains("known.example"));
        assert!(!output.text.contains("bob"));
    }

    #[test]
    fn keeps_host_placeholders_stable_and_distinct_within_one_redactor() {
        let redactor = Redactor::new(RedactionContext {
            hosts: vec!["z.example".into(), "a.example".into()],
            ..Default::default()
        });
        let output = redactor.redact_text("a.example z.example a.example");
        assert_eq!(output.text, "<HOST:1> <HOST:2> <HOST:1>");
    }

    #[test]
    fn redacts_known_secrets_loaded_from_the_vault() {
        let temp = tempfile::tempdir().unwrap();
        let redactor = Redactor::from_storage_directory_with_secrets(
            &temp.path().join("logs"),
            &["vault-token-123".into()],
        );
        let output = redactor.redact_text("token=vault-token-123");
        assert_eq!(output.text, "token=<SECRET>");
        assert!(output.redacted);
    }
}
