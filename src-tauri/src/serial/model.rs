use std::fmt;

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SerialPortInfo {
    pub id: String,
    pub display_name: String,
    pub path: String,
    pub port_type: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SerialParity {
    None,
    Even,
    Odd,
    Mark,
    Space,
}

impl Default for SerialParity {
    fn default() -> Self {
        Self::None
    }
}

impl SerialParity {
    pub fn as_serialport(self) -> serialport::Parity {
        match self {
            Self::None => serialport::Parity::None,
            Self::Even => serialport::Parity::Even,
            Self::Odd => serialport::Parity::Odd,
            Self::Mark | Self::Space => serialport::Parity::None,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SerialFlowControl {
    None,
    Software,
    Hardware,
}

impl Default for SerialFlowControl {
    fn default() -> Self {
        Self::None
    }
}

impl SerialFlowControl {
    pub fn as_serialport(self) -> serialport::FlowControl {
        match self {
            Self::None => serialport::FlowControl::None,
            Self::Software => serialport::FlowControl::Software,
            Self::Hardware => serialport::FlowControl::Hardware,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialReconnectPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_reconnect_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_reconnect_delay")]
    pub max_delay_ms: u64,
}

impl Default for SerialReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_attempts: default_reconnect_attempts(),
            max_delay_ms: default_reconnect_delay(),
        }
    }
}

fn default_reconnect_attempts() -> u32 {
    5
}

fn default_reconnect_delay() -> u64 {
    30_000
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialOpenRequest {
    pub profile_id: String,
    pub connection_id: String,
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: f32,
    #[serde(default)]
    pub parity: SerialParity,
    #[serde(default)]
    pub flow_control: SerialFlowControl,
    #[serde(default)]
    pub read_timeout_ms: u64,
    #[serde(default)]
    pub reconnect: SerialReconnectPolicy,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSessionInfo {
    pub id: String,
    pub profile_id: String,
    pub port: String,
    pub stable_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSessionIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialWriteRequest {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSignalRequest {
    pub id: String,
    pub signal: SerialSignal,
    pub value: bool,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SerialSignal {
    RequestToSend,
    DataTerminalReady,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSignalState {
    pub clear_to_send: bool,
    pub data_set_ready: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialOutputEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialConnectionStateEvent {
    pub id: String,
    pub connection_id: String,
    pub profile_id: String,
    pub state: String,
    pub path: Option<String>,
    pub error: Option<String>,
}

impl fmt::Display for SerialSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestToSend => f.write_str("requestToSend"),
            Self::DataTerminalReady => f.write_str("dataTerminalReady"),
        }
    }
}
