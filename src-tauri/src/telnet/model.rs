#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetConnectRequest {
    pub profile_id: String,
    #[serde(default)]
    pub connection_id: Option<String>,
    pub host: String,
    pub port: u16,
    #[serde(default = "default_terminal_type")]
    pub terminal_type: String,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_ms: u64,
    #[serde(default)]
    pub local_echo: bool,
    #[serde(default)]
    pub keepalive: Option<TelnetKeepaliveOptions>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetKeepaliveOptions {
    pub interval_ms: u64,
    pub max_count: u32,
}

fn default_terminal_type() -> String {
    "xterm-256color".into()
}

fn default_connect_timeout() -> u64 {
    10_000
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetSessionInfo {
    pub id: String,
    pub profile_id: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetSessionIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetWriteRequest {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelnetResizeRequest {
    pub id: String,
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelnetOutputEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelnetExitEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelnetMessageEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelnetEchoEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub force_echo: bool,
}
