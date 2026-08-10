use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectRequest {
    pub profile_id: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub auth: Vec<AuthMethodRef>,
    pub terminal: TerminalRequest,
    pub keepalive: Option<KeepaliveOptions>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub x11: bool,
    #[serde(default)]
    pub x11_display: Option<String>,
    #[serde(default)]
    pub agent_forward: bool,
    #[serde(default)]
    pub jump_chain: Vec<SshJumpRequest>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshJumpRequest {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub auth: Vec<AuthMethodRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SshForwardingType {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshForwardingRequest {
    pub session_id: String,
    pub kind: SshForwardingType,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_address: String,
    pub target_port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshForwardingIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshForwardingInfo {
    pub id: String,
    pub session_id: String,
    pub kind: SshForwardingType,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_address: String,
    pub target_port: u16,
    pub status: SshForwardingStatus,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SshForwardingStatus {
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AuthMethodRef {
    Password {
        #[serde(rename = "secretRef")]
        secret_ref: String,
    },
    PrivateKey {
        #[serde(rename = "fileRef")]
        file_ref: String,
        #[serde(rename = "passphraseRef")]
        passphrase_ref: Option<String>,
    },
    Agent {
        socket: Option<String>,
    },
    KeyboardInteractive,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRequest {
    pub term: String,
    pub columns: u32,
    pub rows: u32,
    pub pixel_width: Option<u32>,
    pub pixel_height: Option<u32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepaliveOptions {
    pub interval_ms: u64,
    pub max_count: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshSessionInfo {
    pub id: String,
    pub profile_id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub request_id: String,
    pub connection_id: String,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint_sha256: String,
    pub status: HostKeyStatus,
    pub previous_fingerprints: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKeyStatus {
    Unknown,
    Changed,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyDecisionRequest {
    pub request_id: String,
    pub decision: HostKeyDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKeyDecision {
    Once,
    Save,
    Reject,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshSessionIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshWriteRequest {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshResizeRequest {
    pub id: String,
    pub columns: u32,
    pub rows: u32,
    pub pixel_width: Option<u32>,
    pub pixel_height: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshOutputEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub data: Vec<u8>,
    pub extended: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshExitEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshErrorEvent {
    pub id: String,
    pub code: String,
    pub details: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshAuthPrompt {
    pub request_id: String,
    pub id: String,
    pub connection_id: String,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<SshAuthPromptItem>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshAuthPromptItem {
    pub text: String,
    pub echo: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshAuthResponseRequest {
    pub request_id: String,
    pub responses: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("invalid SSH request: {0}")]
    InvalidRequest(String),
    #[error("DNS or TCP connection failed")]
    Connection,
    #[error("SSH host key was rejected")]
    HostKeyRejected,
    #[error("SSH host key changed")]
    HostKeyChanged,
    #[error("SSH authentication was rejected")]
    AuthenticationRejected,
    #[error("SSH private key could not be parsed")]
    KeyParse,
    #[error("SSH shell channel could not be opened")]
    ChannelOpen,
    #[error("SSH session was closed")]
    Closed,
    #[error("SSH operation timed out")]
    Timeout,
    #[error("SSH internal operation failed")]
    Internal,
}

impl SshError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalidRequest",
            Self::Connection => "connection",
            Self::HostKeyRejected => "hostKeyRejected",
            Self::HostKeyChanged => "hostKeyChanged",
            Self::AuthenticationRejected => "authenticationRejected",
            Self::KeyParse => "keyParse",
            Self::ChannelOpen => "channelOpen",
            Self::Closed => "closed",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }
}
