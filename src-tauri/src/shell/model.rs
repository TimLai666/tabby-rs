use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ShellType {
    Unix,
    Powershell,
    Cmd,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectedShell {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_type: Option<ShellType>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectShellsRequest {
    #[serde(default)]
    pub identification: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShellDetectionResult {
    pub shells: Vec<DetectedShell>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareSpawnRequest {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub profile_environment: BTreeMap<String, String>,
    #[serde(default)]
    pub runtime_environment: BTreeMap<String, String>,
    #[serde(default)]
    pub shell_type: Option<ShellType>,
    #[serde(default)]
    pub login_shell: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreparedSpawnRequest {
    pub executable: String,
    pub arguments: Vec<String>,
    pub cwd: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub shell_type: Option<ShellType>,
    pub login_shell: bool,
    pub cwd_fallback: bool,
}

impl DetectedShell {
    pub fn new(
        provider_id: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            fs_base: None,
            cwd: None,
            icon: None,
            shell_type: None,
            hidden: false,
            metadata: serde_json::Value::Null,
        }
    }
}
