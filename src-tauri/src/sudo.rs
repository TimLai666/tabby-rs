use std::time::{Duration, Instant};

use rand::{rngs::OsRng, RngCore};

const MAX_PROMPT_BUFFER_BYTES: usize = 4 * 1024;
const MAX_PROMPT_LINE_CHARS: usize = 512;
const PROMPT_TTL: Duration = Duration::from_secs(30);
const SECRET_REF_PREFIX: &str = "vault:profile:";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SudoConfig {
    pub enabled: bool,
    #[serde(default)]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SudoPromptEvent {
    pub session_id: String,
    pub prompt_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SudoPromptError {
    #[error("sudo prompt has expired")]
    Expired,
    #[error("sudo prompt has already been handled")]
    AlreadyHandled,
    #[error("sudo secret reference is invalid")]
    InvalidSecretReference,
}

#[derive(Debug)]
struct PendingPrompt {
    id: String,
    account: Option<String>,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct SudoPromptBroker {
    enabled: bool,
    secret_ref: Option<String>,
    tail: Vec<u8>,
    pending: Option<PendingPrompt>,
    handled: bool,
}

impl SudoPromptBroker {
    pub fn new(config: Option<SudoConfig>) -> Self {
        let enabled = config
            .as_ref()
            .map(|config| config.enabled)
            .unwrap_or(false);
        let secret_ref = config
            .and_then(|config| config.secret_ref)
            .filter(|reference| valid_secret_ref(reference));
        Self {
            enabled: enabled && secret_ref.is_some(),
            secret_ref,
            tail: Vec::new(),
            pending: None,
            handled: false,
        }
    }

    pub fn inspect(&mut self, session_id: &str, bytes: &[u8]) -> Option<SudoPromptEvent> {
        if !self.enabled || self.handled || self.pending.is_some() || bytes.is_empty() {
            return None;
        }
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > MAX_PROMPT_BUFFER_BYTES {
            let excess = self.tail.len() - MAX_PROMPT_BUFFER_BYTES;
            self.tail.drain(..excess);
        }

        let text = String::from_utf8_lossy(&self.tail);
        let visible = strip_ansi(&text);
        let line = visible
            .rsplit(['\n', '\r'])
            .find(|line| !line.trim().is_empty())?
            .trim();
        let account = detect_prompt(line)?;
        let id = new_prompt_id();
        self.pending = Some(PendingPrompt {
            id: id.clone(),
            account: account.clone(),
            expires_at: Instant::now() + PROMPT_TTL,
        });
        Some(SudoPromptEvent {
            session_id: session_id.to_owned(),
            prompt_id: id,
            account,
        })
    }

    pub fn claim(&mut self, prompt_id: &str) -> Result<Option<String>, SudoPromptError> {
        if self.handled {
            return Err(SudoPromptError::AlreadyHandled);
        }
        let Some(pending) = self.pending.as_ref() else {
            return Ok(None);
        };
        if pending.id != prompt_id {
            return Ok(None);
        }
        if Instant::now() >= pending.expires_at {
            self.pending = None;
            self.handled = true;
            return Err(SudoPromptError::Expired);
        }
        self.pending = None;
        self.handled = true;
        self.secret_ref
            .clone()
            .ok_or(SudoPromptError::InvalidSecretReference)
            .map(Some)
    }

    #[cfg(test)]
    fn pending_account(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .and_then(|prompt| prompt.account.as_deref())
    }
}

pub fn profile_id_from_secret_ref(reference: &str) -> Result<&str, SudoPromptError> {
    let profile_id = reference
        .strip_prefix(SECRET_REF_PREFIX)
        .ok_or(SudoPromptError::InvalidSecretReference)?;
    if profile_id.is_empty() || profile_id.len() > 512 || profile_id.chars().any(char::is_control) {
        return Err(SudoPromptError::InvalidSecretReference);
    }
    Ok(profile_id)
}

fn valid_secret_ref(reference: &str) -> bool {
    profile_id_from_secret_ref(reference).is_ok()
}

fn detect_prompt(line: &str) -> Option<Option<String>> {
    if line.chars().count() > MAX_PROMPT_LINE_CHARS
        || line
            .chars()
            .any(|character| character.is_control() && character != '\t')
    {
        return None;
    }
    let normalized = line.trim().to_lowercase();
    let body = normalized
        .strip_suffix(':')
        .or_else(|| normalized.strip_suffix('：'))?
        .trim_end();

    if let Some(rest) = body.strip_prefix("[sudo: authenticate]") {
        return (!rest.trim().is_empty()).then_some(None);
    }
    let body = body.strip_prefix("[sudo] ")?.trim();

    const PREFIXES: &[&str] = &[
        "password for ",
        "passwort für ",
        "mot de passe de ",
        "contraseña para ",
        "senha para ",
        "password di ",
        "пароль для ",
        "hasło użytkownika ",
        "heslo pro ",
        "lösenord för ",
        "adgangskode for ",
        "kata sandi untuk ",
        "пароль до ",
        "lozinka za ",
    ];
    for prefix in PREFIXES {
        if let Some(account) = body.strip_prefix(prefix) {
            return valid_account(account).then(|| Some(account.to_owned()));
        }
    }

    const SUFFIXES: &[&str] = &[
        " 的密码",
        " 的密碼",
        " のパスワード",
        " 암호",
        " için parola",
    ];
    for suffix in SUFFIXES {
        if let Some(account) = body.strip_suffix(suffix) {
            return valid_account(account).then(|| Some(account.to_owned()));
        }
    }
    None
}

fn valid_account(account: &str) -> bool {
    let account = account.trim();
    !account.is_empty()
        && account.len() <= 256
        && !account.contains(':')
        && !account.chars().any(char::is_control)
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            output.push(character);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            chars.next();
        }
    }
    output
}

fn new_prompt_id() -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::{detect_prompt, profile_id_from_secret_ref, SudoConfig, SudoPromptBroker};

    fn broker() -> SudoPromptBroker {
        SudoPromptBroker::new(Some(SudoConfig {
            enabled: true,
            secret_ref: Some("vault:profile:local:default".into()),
        }))
    }

    #[test]
    fn detects_bounded_multilingual_prompts_split_across_chunks() {
        let mut broker = broker();
        assert!(broker.inspect("session", b"[su").is_none());
        let event = broker
            .inspect("session", "do] 使用者 的密碼：".as_bytes())
            .unwrap();
        assert_eq!(event.session_id, "session");
        assert_eq!(broker.pending_account(), Some("使用者"));
    }

    #[test]
    fn detects_sudo_rs_without_guessing_an_account() {
        assert_eq!(detect_prompt("[sudo: authenticate] Password:"), Some(None));
    }

    #[test]
    fn rejects_forged_or_overbroad_password_text() {
        assert_eq!(detect_prompt("password for root:"), None);
        assert_eq!(detect_prompt("notice: [sudo] password for root:"), None);
        assert_eq!(detect_prompt("[sudo] password for root: run this"), None);
    }

    #[test]
    fn a_prompt_can_only_be_claimed_once() {
        let mut broker = broker();
        let event = broker
            .inspect("session", b"[sudo] password for alice:")
            .unwrap();
        assert_eq!(
            broker.claim(&event.prompt_id).unwrap(),
            Some("vault:profile:local:default".into())
        );
        assert!(broker.claim(&event.prompt_id).is_err());
    }

    #[test]
    fn secret_references_are_strictly_scoped_to_profile_vault_entries() {
        assert_eq!(
            profile_id_from_secret_ref("vault:profile:local:default").unwrap(),
            "local:default"
        );
        assert!(profile_id_from_secret_ref("keychain:anything").is_err());
        assert!(profile_id_from_secret_ref("vault:profile:").is_err());
    }
}
