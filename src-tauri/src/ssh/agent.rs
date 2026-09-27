use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Auto,
    Pageant,
    Pipe,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSocketRequest {
    #[serde(default)]
    pub agent_type: Option<AgentType>,
    #[serde(default)]
    pub agent_path: Option<String>,
}

const DEFAULT_PIPE_PATH: &str = r"\\.\pipe\openssh-ssh-agent";

#[cfg(windows)]
fn probe_default_pipe_exists() -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SEM_TIMEOUT;
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let wide: Vec<u16> = OsStr::new(DEFAULT_PIPE_PATH)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe { WaitNamedPipeW(wide.as_ptr(), 1) };
    if result != 0 {
        return true;
    }
    let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    error == ERROR_SEM_TIMEOUT
}

fn resolve_agent_socket_internal(
    request: AgentSocketRequest,
    is_windows: bool,
    probe_default: impl FnOnce() -> bool,
) -> Option<String> {
    let agent_type = request.agent_type.unwrap_or(AgentType::Auto);

    if is_windows {
        match agent_type {
            AgentType::Pipe => {
                if let Some(path) = request.agent_path {
                    if !path.is_empty() {
                        return Some(path);
                    }
                }
                Some(DEFAULT_PIPE_PATH.to_owned())
            }
            AgentType::Pageant => None,
            AgentType::Auto => {
                if probe_default() {
                    Some(DEFAULT_PIPE_PATH.to_owned())
                } else {
                    None
                }
            }
        }
    } else {
        request
            .agent_path
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
    }
}

pub fn resolve_agent_socket(request: AgentSocketRequest) -> Option<String> {
    #[cfg(windows)]
    {
        resolve_agent_socket_internal(request, true, probe_default_pipe_exists)
    }
    #[cfg(not(windows))]
    {
        resolve_agent_socket_internal(request, false, || false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_default_exists() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: Some("custom".into()),
        };
        let result = resolve_agent_socket_internal(request, true, || true);
        assert_eq!(result, Some(DEFAULT_PIPE_PATH.to_owned()));
    }

    #[test]
    fn windows_default_missing() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: Some("custom".into()),
        };
        let result = resolve_agent_socket_internal(request, true, || false);
        assert_eq!(result, None);
    }

    #[test]
    fn windows_explicit_pipe_default() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pipe),
            agent_path: None,
        };
        let result = resolve_agent_socket_internal(request, true, || false);
        assert_eq!(result, Some(DEFAULT_PIPE_PATH.to_owned()));
    }

    #[test]
    fn windows_explicit_pipe_custom_preserves_whitespace() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pipe),
            agent_path: Some("  \\\\.\\pipe\\custom  ".into()),
        };
        let result = resolve_agent_socket_internal(request, true, || false);
        assert_eq!(result, Some("  \\\\.\\pipe\\custom  ".into()));
    }

    #[test]
    fn windows_forced_pageant_no_probe() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pageant),
            agent_path: Some("ignored".into()),
        };
        let mut probed = false;
        let result = resolve_agent_socket_internal(request, true, || {
            probed = true;
            true
        });
        assert_eq!(result, None);
        assert!(!probed, "Pageant mode must not probe");
    }

    #[test]
    fn unix_custom_trim_env_fallback() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: Some("  /tmp/agent  ".into()),
        };
        let result = resolve_agent_socket_internal(request, false, || true);
        assert_eq!(result, Some("/tmp/agent".into()));
    }

    #[test]
    fn unix_blank_path_returns_none() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: Some("   ".into()),
        };
        let result = resolve_agent_socket_internal(request, false, || true);
        assert_eq!(result, None);
    }

    #[test]
    fn unix_absent_path_returns_none() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: None,
        };
        let result = resolve_agent_socket_internal(request, false, || true);
        assert_eq!(result, None);
    }

    #[test]
    fn unix_mode_ignored() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pageant),
            agent_path: Some("  /tmp/agent  ".into()),
        };
        let result = resolve_agent_socket_internal(request, false, || true);
        assert_eq!(result, Some("/tmp/agent".into()));
    }

    #[test]
    fn serde_camel_case_and_invalid_mode() {
        let json = r#"{"agentType":"auto","agentPath":"/tmp/sock"}"#;
        let req: AgentSocketRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_type, Some(AgentType::Auto));
        assert_eq!(req.agent_path, Some("/tmp/sock".into()));

        let json = r#"{"agentType":"invalid"}"#;
        assert!(serde_json::from_str::<AgentSocketRequest>(json).is_err());
    }

    #[test]
    fn omitted_agent_type_defaults_auto() {
        let request = AgentSocketRequest {
            agent_type: None,
            agent_path: None,
        };
        let mut probed = false;
        let result = resolve_agent_socket_internal(request, true, || {
            probed = true;
            true
        });
        assert_eq!(result, Some(DEFAULT_PIPE_PATH.to_owned()));
        assert!(probed, "Auto mode must probe when agent_type omitted");

        let req: AgentSocketRequest = serde_json::from_str("{}").unwrap();
        let mut probed2 = false;
        let result2 = resolve_agent_socket_internal(req, true, || {
            probed2 = true;
            true
        });
        assert_eq!(result2, Some(DEFAULT_PIPE_PATH.to_owned()));
        assert!(probed2, "Empty JSON object must deserialize to Auto mode");

        let req: AgentSocketRequest = serde_json::from_str(r#"{"agentType":null,"agentPath":null}"#).unwrap();
        let mut probed3 = false;
        let result3 = resolve_agent_socket_internal(req, true, || {
            probed3 = true;
            true
        });
        assert_eq!(result3, Some(DEFAULT_PIPE_PATH.to_owned()));
        assert!(probed3, "Explicit null fields must deserialize to Auto mode");
    }

    #[test]
    fn empty_pipe_path_defaults() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pipe),
            agent_path: Some("".into()),
        };
        let mut probed = false;
        let result = resolve_agent_socket_internal(request, true, || {
            probed = true;
            true
        });
        assert_eq!(result, Some(DEFAULT_PIPE_PATH.to_owned()));
        assert!(!probed, "Explicit Pipe must not probe");
    }

    #[test]
    fn unix_never_calls_probe() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Auto),
            agent_path: Some("/tmp/agent".into()),
        };
        let result = resolve_agent_socket_internal(request, false, || {
            panic!("Unix must never call probe");
        });
        assert_eq!(result, Some("/tmp/agent".into()));
    }

    #[test]
    fn windows_explicit_pipe_never_calls_probe() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pipe),
            agent_path: Some("\\\\.\\pipe\\custom".into()),
        };
        let result = resolve_agent_socket_internal(request, true, || {
            panic!("Explicit Pipe must never call probe");
        });
        assert_eq!(result, Some("\\\\.\\pipe\\custom".into()));
    }

    #[test]
    fn windows_explicit_pageant_never_calls_probe_panic() {
        let request = AgentSocketRequest {
            agent_type: Some(AgentType::Pageant),
            agent_path: Some("ignored".into()),
        };
        let result = resolve_agent_socket_internal(request, true, || {
            panic!("Explicit Pageant must never call probe");
        });
        assert_eq!(result, None);
    }
}
