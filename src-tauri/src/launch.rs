use std::{collections::BTreeMap, env, path::PathBuf};

use url::Url;

use crate::identity::URL_SCHEME;

const MAX_ARGUMENTS: usize = 128;
const MAX_ARGUMENT_LENGTH: usize = 32 * 1024;
const MAX_URL_LENGTH: usize = 16 * 1024;
const MAX_SCALAR_LENGTH: usize = 4 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    pub profile: Option<String>,
    pub cwd: Option<String>,
    pub new_window: bool,
    pub safe_mode: bool,
    pub config: Option<String>,
    pub command: Vec<String>,
    pub urls: Vec<String>,
    pub argv: LegacyCliArguments,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyCliArguments {
    #[serde(rename = "_")]
    pub commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub escape: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub debug: bool,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_number: Option<u64>,
    pub new_window: bool,
    pub safe_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchContext {
    pub request: LaunchRequest,
    pub cwd: String,
    pub second_instance: bool,
    pub parse_error: Option<String>,
}

pub fn initial_launch_context() -> LaunchContext {
    let argv = env::args().collect::<Vec<_>>();
    let cwd = env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .into_owned();
    parse_launch_context(&argv, cwd, false)
}

pub fn parse_launch_context(argv: &[String], cwd: String, second_instance: bool) -> LaunchContext {
    match parse_launch_request(argv, &cwd) {
        Ok(request) => LaunchContext {
            request,
            cwd,
            second_instance,
            parse_error: None,
        },
        Err(error) => LaunchContext {
            request: LaunchRequest::default(),
            cwd,
            second_instance,
            parse_error: Some(error),
        },
    }
}

fn parse_launch_request(argv: &[String], cwd: &str) -> Result<LaunchRequest, String> {
    if argv.len().saturating_sub(1) > MAX_ARGUMENTS {
        return Err(format!("too many arguments (maximum {MAX_ARGUMENTS})"));
    }
    for argument in argv {
        validate_argument(argument)?;
    }

    let mut request = LaunchRequest::default();
    let mut tokens = Vec::new();
    for token in argv.iter().skip(1) {
        if token.starts_with("-psn_") {
            continue;
        }
        if is_tabby_rs_url(token) {
            let parsed = parse_tabby_rs_url(token)?;
            merge_url_request(&mut request, parsed)?;
            request.urls.push(token.clone());
        } else {
            tokens.push(token.clone());
        }
    }

    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        match token {
            "--profile" => {
                index += 1;
                let value = required_value(&tokens, index, "--profile")?;
                set_scalar(&mut request.profile, value, "profile")?;
            }
            value if value.starts_with("--profile=") => {
                set_scalar(
                    &mut request.profile,
                    value.trim_start_matches("--profile=").to_owned(),
                    "profile",
                )?;
            }
            "--cwd" => {
                index += 1;
                let value = required_value(&tokens, index, "--cwd")?;
                set_scalar(&mut request.cwd, value, "cwd")?;
            }
            value if value.starts_with("--cwd=") => {
                set_scalar(
                    &mut request.cwd,
                    value.trim_start_matches("--cwd=").to_owned(),
                    "cwd",
                )?;
            }
            "--config" => {
                index += 1;
                let value = required_value(&tokens, index, "--config")?;
                set_scalar(&mut request.config, value, "config")?;
            }
            value if value.starts_with("--config=") => {
                set_scalar(
                    &mut request.config,
                    value.trim_start_matches("--config=").to_owned(),
                    "config",
                )?;
            }
            "--new-window" => request.new_window = true,
            "--safe-mode" => request.safe_mode = true,
            "--debug" | "-d" => request.argv.debug = true,
            "--hidden" => request.argv.hidden = true,
            "--" => {
                request.command = tokens[(index + 1)..].to_vec();
                break;
            }
            "open" => {
                request.argv.commands = vec!["open".into()];
                let directory = tokens
                    .get(index + 1)
                    .cloned()
                    .unwrap_or_else(|| cwd.to_owned());
                validate_scalar(&directory, "directory")?;
                set_scalar(&mut request.cwd, directory.clone(), "cwd")?;
                request.argv.directory = Some(directory);
                break;
            }
            "run" | "/k" => {
                request.argv.commands = vec!["run".into()];
                request.command = tokens[(index + 1)..].to_vec();
                break;
            }
            "profile" => {
                request.argv.commands = vec!["profile".into()];
                let value = required_value(&tokens, index + 1, "profile")?;
                set_scalar(&mut request.profile, value, "profile")?;
                break;
            }
            "paste" => {
                request.argv.commands = vec!["paste".into()];
                let mut values = tokens[(index + 1)..].to_vec();
                if values
                    .first()
                    .is_some_and(|value| value == "-e" || value == "--escape")
                {
                    request.argv.escape = true;
                    values.remove(0);
                }
                if !values.is_empty() {
                    request.argv.text = Some(values.join(" "));
                }
                break;
            }
            "recent" => {
                request.argv.commands = vec!["recent".into()];
                let value = required_value(&tokens, index + 1, "recent")?;
                request.argv.profile_number = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "recent index must be a non-negative integer".to_owned())?,
                );
                break;
            }
            "quickConnect" => {
                request.argv.commands = vec!["quickConnect".into()];
                request.argv.provider_id = Some(required_value(
                    &tokens,
                    index + 1,
                    "quickConnect providerId",
                )?);
                request.argv.query =
                    Some(required_value(&tokens, index + 2, "quickConnect query")?);
                if tokens.len() != index + 3 {
                    return Err("quickConnect accepts exactly providerId and query".into());
                }
                break;
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            _ => {
                request.argv.commands = vec!["run".into()];
                request.command = tokens[index..].to_vec();
                break;
            }
        }
        index += 1;
    }

    request.argv.profile_name = request.profile.clone();
    if request.argv.directory.is_none() {
        request.argv.directory = request.cwd.clone();
    }
    request.argv.command = request.command.clone();
    request.argv.new_window = request.new_window;
    request.argv.safe_mode = request.safe_mode;
    request.argv.config = request.config.clone();

    if request.argv.commands.is_empty() {
        if !request.command.is_empty() {
            request.argv.commands.push("run".into());
        } else if request.profile.is_some() {
            request.argv.commands.push("profile".into());
        } else if request.cwd.is_some() {
            request.argv.commands.push("open".into());
        }
    }

    Ok(request)
}

fn parse_tabby_rs_url(input: &str) -> Result<LaunchRequest, String> {
    if input.len() > MAX_URL_LENGTH {
        return Err(format!("URL exceeds {MAX_URL_LENGTH} bytes"));
    }
    let parsed = Url::parse(input).map_err(|error| format!("invalid URL: {error}"))?;
    if parsed.scheme() != URL_SCHEME {
        return Err(format!("unsupported URL scheme: {}", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() || parsed.port().is_some() {
        return Err("URL credentials and ports are not allowed".into());
    }
    if parsed.fragment().is_some() {
        return Err("URL fragments are not allowed".into());
    }

    let action = parsed
        .host_str()
        .ok_or_else(|| "URL action is missing".to_owned())?
        .to_ascii_lowercase();
    let query = collect_query(&parsed)?;
    let mut request = LaunchRequest::default();

    match action.as_str() {
        "open" => {
            reject_unknown_query(&query, &["profile", "cwd"])?;
            if let Some(profile) = query.get("profile") {
                set_scalar(&mut request.profile, profile.clone(), "profile")?;
            }
            if let Some(cwd) = query.get("cwd") {
                set_scalar(&mut request.cwd, cwd.clone(), "cwd")?;
            }
            if request.profile.is_none() && request.cwd.is_none() {
                return Err("open requires profile or cwd".into());
            }
        }
        "ssh" => {
            reject_unknown_query(&query, &[])?;
            let path = parsed.path().trim_matches('/');
            if path.is_empty() || path.contains('/') {
                return Err("ssh URL requires exactly one profile id path segment".into());
            }
            let profile = percent_decode_component(path)?;
            set_scalar(&mut request.profile, profile, "profile")?;
        }
        "local" => {
            reject_unknown_query(&query, &["cwd"])?;
            let cwd = query
                .get("cwd")
                .cloned()
                .ok_or_else(|| "local URL requires cwd".to_owned())?;
            set_scalar(&mut request.cwd, cwd, "cwd")?;
        }
        _ => return Err(format!("unknown URL action: {action}")),
    }

    Ok(request)
}

fn collect_query(url: &Url) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        validate_scalar(&key, "query key")?;
        validate_scalar(&value, &key)?;
        if result
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("duplicate URL query parameter".into());
        }
    }
    Ok(result)
}

fn reject_unknown_query(query: &BTreeMap<String, String>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = query.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unknown URL query parameter: {key}"));
    }
    Ok(())
}

fn merge_url_request(target: &mut LaunchRequest, source: LaunchRequest) -> Result<(), String> {
    if let Some(profile) = source.profile {
        set_scalar(&mut target.profile, profile, "profile")?;
    }
    if let Some(cwd) = source.cwd {
        set_scalar(&mut target.cwd, cwd, "cwd")?;
    }
    if !source.command.is_empty() {
        if !target.command.is_empty() && target.command != source.command {
            return Err("conflicting command values".into());
        }
        target.command = source.command;
    }
    Ok(())
}

fn set_scalar(target: &mut Option<String>, value: String, field: &str) -> Result<(), String> {
    validate_scalar(&value, field)?;
    if let Some(existing) = target {
        if existing != &value {
            return Err(format!("conflicting {field} values"));
        }
    } else {
        *target = Some(value);
    }
    Ok(())
}

fn validate_argument(value: &str) -> Result<(), String> {
    if value.len() > MAX_ARGUMENT_LENGTH {
        return Err(format!("argument exceeds {MAX_ARGUMENT_LENGTH} bytes"));
    }
    if value.contains('\0') {
        return Err("argument contains NUL".into());
    }
    Ok(())
}

fn validate_scalar(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    if value.len() > MAX_SCALAR_LENGTH {
        return Err(format!("{field} exceeds {MAX_SCALAR_LENGTH} bytes"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{field} contains control characters"));
    }
    Ok(())
}

fn required_value(tokens: &[String], index: usize, name: &str) -> Result<String, String> {
    let value = tokens
        .get(index)
        .cloned()
        .ok_or_else(|| format!("{name} requires a value"))?;
    validate_scalar(&value, name)?;
    Ok(value)
}

fn is_tabby_rs_url(value: &str) -> bool {
    value
        .get(..(URL_SCHEME.len() + 3))
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&format!("{URL_SCHEME}://")))
}

fn percent_decode_component(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("incomplete percent escape".into());
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "URL path is not valid UTF-8".into())
}

fn hex_value(value: u8) -> Result<u8, String> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid percent escape".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_launch_context, LaunchRequest};

    fn parse(args: &[&str]) -> LaunchRequest {
        let argv = args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>();
        let context = parse_launch_context(&argv, "/work".into(), false);
        assert_eq!(context.parse_error, None);
        context.request
    }

    #[test]
    fn parses_profile_and_cwd_without_shell_reparsing() {
        let request = parse(&[
            "tabby-rs",
            "--profile",
            "PowerShell",
            "--cwd",
            "C:\\工作 目錄",
        ]);
        assert_eq!(request.profile.as_deref(), Some("PowerShell"));
        assert_eq!(request.cwd.as_deref(), Some("C:\\工作 目錄"));
        assert_eq!(request.argv.commands, ["profile"]);
    }

    #[test]
    fn preserves_command_arguments_as_an_array() {
        let request = parse(&["tabby-rs", "run", "printf", "%s", "a; rm -rf /"]);
        assert_eq!(request.command, ["printf", "%s", "a; rm -rf /"]);
        assert_eq!(request.argv.command, request.command);
    }

    #[test]
    fn parses_supported_deep_links() {
        let request = parse(&["tabby-rs", "tabby-rs://open?profile=server-1"]);
        assert_eq!(request.profile.as_deref(), Some("server-1"));

        let request = parse(&["tabby-rs", "tabby-rs://local?cwd=%2Ftmp%2Fhello%20world"]);
        assert_eq!(request.cwd.as_deref(), Some("/tmp/hello world"));

        let request = parse(&["tabby-rs", "tabby-rs://ssh/Profile%20A"]);
        assert_eq!(request.profile.as_deref(), Some("Profile A"));
    }

    #[test]
    fn rejects_unknown_and_ambiguous_deep_links() {
        for url in [
            "tabby-rs://exec?command=rm",
            "tabby-rs://open?profile=a&profile=b",
            "tabby-rs://local?cwd=%0Aevil",
        ] {
            let argv = vec!["tabby-rs".to_owned(), url.to_owned()];
            let context = parse_launch_context(&argv, "/work".into(), false);
            assert!(context.parse_error.is_some(), "{url} should be rejected");
        }
    }

    #[test]
    fn parses_second_instance_metadata() {
        let argv = vec!["tabby-rs".to_owned(), "--safe-mode".to_owned()];
        let context = parse_launch_context(&argv, "/work".into(), true);
        assert!(context.second_instance);
        assert!(context.request.safe_mode);
        assert!(context.request.argv.safe_mode);
    }
}
