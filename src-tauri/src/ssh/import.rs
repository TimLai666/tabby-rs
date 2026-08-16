use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::error::AppError;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_INCLUDE_DEPTH: usize = 16;

pub fn private_key_candidates() -> Vec<String> {
    let Some(home) = home_directory() else {
        return Vec::new();
    };
    let directory = PathBuf::from(home).join(".ssh");
    let mut candidates = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            is_private_key_filename(name).then(|| entry.path().to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
}

fn is_private_key_filename(name: &str) -> bool {
    name.strip_prefix("id_")
        .map(|suffix| {
            !suffix.is_empty()
                && suffix
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportSource {
    pub path: String,
    #[serde(default)]
    pub existing_profile_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshImportProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub private_keys: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportConflict {
    pub profile_id: String,
    pub profile_name: String,
    pub existing_profile_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportPreview {
    pub source: String,
    pub revision: Option<String>,
    pub profiles: Vec<SshImportProfile>,
    pub conflicts: Vec<SshImportConflict>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshImportAction {
    Skip,
    Duplicate,
    Overwrite,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportSelection {
    pub path: String,
    pub expected_revision: Option<String>,
    pub selections: Vec<SshImportSelectionItem>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportSelectionItem {
    pub profile_id: String,
    pub action: SshImportAction,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportFailure {
    pub profile_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshImportReport {
    pub imported: Vec<SshImportProfile>,
    pub skipped: Vec<String>,
    pub failed: Vec<SshImportFailure>,
    pub revision: String,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
struct HostBlock {
    patterns: Vec<String>,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    private_keys: Vec<String>,
}

pub fn preview(source: &SshImportSource) -> Result<SshImportPreview, AppError> {
    let path = validate_source_path(&source.path)?;
    let profiles = parse_source(&path)?;
    let existing = source
        .existing_profile_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let conflicts = profiles
        .iter()
        .filter(|profile| existing.contains(&profile.id))
        .map(|profile| SshImportConflict {
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            existing_profile_id: profile.id.clone(),
        })
        .collect();
    Ok(SshImportPreview {
        source: path.to_string_lossy().into_owned(),
        revision: None,
        profiles,
        conflicts,
    })
}

pub fn apply(
    config_path: &Path,
    selection: SshImportSelection,
    current_yaml: &str,
    current_revision: Option<&str>,
) -> Result<
    (
        String,
        Vec<SshImportProfile>,
        Vec<String>,
        Vec<SshImportFailure>,
    ),
    AppError,
> {
    if selection.path.is_empty() {
        return Err(AppError::InvalidArgument("SSH import path is empty".into()));
    }
    if let Some(expected) = selection.expected_revision.as_deref() {
        if current_revision != Some(expected) {
            return Err(AppError::Conflict(
                "config.yaml changed since the SSH import preview".into(),
            ));
        }
    }

    let source = SshImportSource {
        path: selection.path,
        existing_profile_ids: Vec::new(),
    };
    let profiles = preview(&source)?.profiles;
    let mut by_id = profiles
        .into_iter()
        .map(|profile| (profile.id.clone(), profile))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut root: serde_yaml::Value = if current_yaml.trim().is_empty() {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    } else {
        serde_yaml::from_str(current_yaml)?
    };
    let mapping = root
        .as_mapping_mut()
        .ok_or_else(|| AppError::InvalidData("config.yaml root must be a mapping".into()))?;
    let profiles_key = serde_yaml::Value::String("profiles".into());
    let profile_list = mapping
        .entry(profiles_key.clone())
        .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()))
        .as_sequence_mut()
        .ok_or_else(|| AppError::InvalidData("config.yaml profiles must be a list".into()))?;
    let mut existing_ids = profile_list
        .iter()
        .filter_map(|item| item.get("id").and_then(serde_yaml::Value::as_str))
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    for item in selection.selections {
        let Some(mut profile) = by_id.remove(&item.profile_id) else {
            failed.push(SshImportFailure {
                profile_id: item.profile_id,
                reason: "profile was not present in the preview source".into(),
            });
            continue;
        };
        let conflict = existing_ids.contains(&profile.id);
        match (conflict, item.action) {
            (true, SshImportAction::Skip) => {
                skipped.push(profile.id);
            }
            (true, SshImportAction::Duplicate) => {
                let base_id = profile.id.clone();
                let mut suffix = 2;
                while existing_ids.contains(&profile.id) {
                    profile.id = format!("{base_id}:imported-{suffix}");
                    suffix += 1;
                }
                append_profile(profile, profile_list, &mut existing_ids, &mut imported)?;
            }
            (true, SshImportAction::Overwrite) => {
                profile_list.retain(|item| {
                    item.get("id").and_then(serde_yaml::Value::as_str) != Some(profile.id.as_str())
                });
                existing_ids.remove(&profile.id);
                append_profile(profile, profile_list, &mut existing_ids, &mut imported)?;
            }
            (false, _) => append_profile(profile, profile_list, &mut existing_ids, &mut imported)?,
        }
    }

    let yaml = serde_yaml::to_string(&root)?;
    let _ = config_path;
    Ok((yaml, imported, skipped, failed))
}

fn append_profile(
    profile: SshImportProfile,
    target: &mut Vec<serde_yaml::Value>,
    existing_ids: &mut HashSet<String>,
    imported: &mut Vec<SshImportProfile>,
) -> Result<(), AppError> {
    existing_ids.insert(profile.id.clone());
    target.push(profile_yaml(&profile));
    imported.push(profile);
    Ok(())
}

fn profile_yaml(profile: &SshImportProfile) -> serde_yaml::Value {
    let mut options = serde_yaml::Mapping::new();
    options.insert("host".into(), profile.host.clone().into());
    options.insert("port".into(), profile.port.into());
    if let Some(user) = profile.user.as_deref() {
        options.insert("user".into(), user.into());
    }
    if !profile.private_keys.is_empty() {
        options.insert("auth".into(), "publicKey".into());
        options.insert(
            "privateKeys".into(),
            serde_yaml::Value::Sequence(
                profile
                    .private_keys
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
            ),
        );
    }
    let mut profile_mapping = serde_yaml::Mapping::new();
    profile_mapping.insert("id".into(), profile.id.clone().into());
    profile_mapping.insert("name".into(), profile.name.clone().into());
    profile_mapping.insert("type".into(), "ssh".into());
    profile_mapping.insert("options".into(), serde_yaml::Value::Mapping(options));
    serde_yaml::Value::Mapping(profile_mapping)
}

pub fn parse_config(path: &Path) -> Result<Vec<SshImportProfile>, AppError> {
    let canonical = fs::canonicalize(path)
        .map_err(|_| AppError::InvalidArgument("SSH config file is unavailable".into()))?;
    let mut visited = HashSet::new();
    let mut blocks = Vec::new();
    parse_file(&canonical, 0, &mut visited, &mut blocks)?;
    let aliases = blocks
        .iter()
        .flat_map(|block| block.patterns.iter())
        .filter(|pattern| {
            !pattern.is_empty()
                && !pattern.starts_with('!')
                && !pattern.contains('*')
                && !pattern.contains('?')
        })
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut profiles = Vec::new();
    for alias in aliases {
        let mut hostname = None;
        let mut user = None;
        let mut port = None;
        let mut private_keys = Vec::new();
        for block in &blocks {
            if !host_block_matches(block, &alias) {
                continue;
            }
            if hostname.is_none() {
                hostname = block.hostname.clone();
            }
            if user.is_none() {
                user = block.user.clone();
            }
            if port.is_none() {
                port = block.port;
            }
            for key in &block.private_keys {
                if !private_keys.contains(key) {
                    private_keys.push(key.clone());
                }
            }
        }
        if hostname.as_deref() == Some("none") {
            continue;
        }
        let host = hostname.unwrap_or_else(|| alias.clone());
        if host.is_empty() || host.len() > 255 || host.chars().any(char::is_control) {
            continue;
        }
        let id = stable_profile_id(&canonical, &alias);
        profiles.push(SshImportProfile {
            id,
            name: format!("{alias} (.ssh/config)"),
            host,
            port: port.unwrap_or(22),
            user,
            private_keys,
        });
    }
    profiles.sort_by(|left, right| left.id.cmp(&right.id));
    profiles.dedup_by(|left, right| left.id == right.id);
    Ok(profiles)
}

fn parse_source(path: &Path) -> Result<Vec<SshImportProfile>, AppError> {
    match parse_static_profiles(path) {
        Ok(profiles) if !profiles.is_empty() => Ok(profiles),
        _ => parse_config(path),
    }
}

fn parse_static_profiles(path: &Path) -> Result<Vec<SshImportProfile>, AppError> {
    let text = fs::read_to_string(path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text)?;
    let Some(items) = value.as_sequence() else {
        return Ok(Vec::new());
    };
    let mut profiles = Vec::new();
    for item in items {
        let Some(name) = item.get("name").and_then(serde_yaml::Value::as_str) else {
            continue;
        };
        let Some(options) = item.get("options") else {
            continue;
        };
        let Some(host) = options.get("host").and_then(serde_yaml::Value::as_str) else {
            continue;
        };
        if host.is_empty() || host.len() > 255 || host.chars().any(char::is_control) {
            continue;
        }
        let port = options
            .get("port")
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .and_then(|value| u16::try_from(value).ok())
            .filter(|port| *port > 0)
            .unwrap_or(22);
        let private_keys = options
            .get("privateKeys")
            .and_then(serde_yaml::Value::as_sequence)
            .into_iter()
            .flatten()
            .filter_map(serde_yaml::Value::as_str)
            .map(str::to_owned)
            .collect();
        profiles.push(SshImportProfile {
            id: stable_static_profile_id(name),
            name: name.to_owned(),
            host: host.to_owned(),
            port,
            user: options
                .get("user")
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned),
            private_keys,
        });
    }
    Ok(profiles)
}

fn host_block_matches(block: &HostBlock, alias: &str) -> bool {
    let positive = block
        .patterns
        .iter()
        .any(|pattern| !pattern.starts_with('!') && wildcard_match(pattern, alias));
    let negative = block.patterns.iter().any(|pattern| {
        pattern
            .strip_prefix('!')
            .map(|pattern| wildcard_match(pattern, alias))
            .unwrap_or(false)
    });
    positive && !negative
}

fn parse_file(
    path: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    blocks: &mut Vec<HostBlock>,
) -> Result<(), AppError> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(AppError::InvalidData(
            "SSH config Include depth is too large".into(),
        ));
    }
    let canonical = fs::canonicalize(path)
        .map_err(|_| AppError::InvalidArgument("SSH config file is unavailable".into()))?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    let bytes = fs::read(&canonical)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(AppError::InvalidData("SSH config file is too large".into()));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| AppError::InvalidData("SSH config file is not UTF-8".into()))?;
    let mut current: Option<HostBlock> = None;
    for raw_line in text.lines() {
        let Some((key, values)) = parse_config_line(raw_line) else {
            continue;
        };
        match key.as_str() {
            "host" => {
                if let Some(block) = current.take() {
                    blocks.push(block);
                }
                current = Some(HostBlock {
                    patterns: values,
                    ..Default::default()
                });
            }
            "include" => {
                let continuation_patterns = current.as_ref().map(|block| block.patterns.clone());
                if let Some(block) = current.take() {
                    blocks.push(block);
                }
                for include in expand_include(&values, canonical.parent().unwrap_or(Path::new(".")))
                {
                    parse_file(&include, depth + 1, visited, blocks)?;
                }
                if let Some(patterns) = continuation_patterns {
                    current = Some(HostBlock {
                        patterns,
                        ..Default::default()
                    });
                }
            }
            "hostname" | "user" | "port" | "identityfile" => {
                let Some(block) = current.as_mut() else {
                    continue;
                };
                match key.as_str() {
                    "hostname" => block.hostname = values.first().cloned(),
                    "user" => block.user = values.first().cloned(),
                    "port" => {
                        block.port = values
                            .first()
                            .and_then(|value| value.parse::<u16>().ok())
                            .filter(|port| *port > 0)
                    }
                    "identityfile" => block.private_keys.extend(values.into_iter().map(|value| {
                        resolve_identity_path(&value, canonical.parent().unwrap_or(Path::new(".")))
                    })),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }
    Ok(())
}

fn parse_config_line(line: &str) -> Option<(String, Vec<String>)> {
    let mut chars = line.chars().peekable();
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut token_started = false;
    let mut quote = None;

    while let Some(character) = chars.next() {
        if character == '\\' {
            let Some(next) = chars.peek().copied() else {
                return None;
            };
            if next.is_whitespace() || matches!(next, '#' | '\\' | '"' | '\'') {
                token.push(next);
                chars.next();
            } else {
                token.push(character);
            }
            token_started = true;
            continue;
        }
        if let Some(quote_character) = quote {
            if character == quote_character {
                quote = None;
            } else {
                token.push(character);
            }
            token_started = true;
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                token_started = true;
            }
            '#' => break,
            character if character.is_whitespace() => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            _ => {
                token.push(character);
                token_started = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if token_started {
        tokens.push(token);
    }
    let (key, values) = tokens.split_first()?;
    if values.is_empty() {
        return None;
    }
    Some((key.to_ascii_lowercase(), values.to_vec()))
}

fn expand_include(values: &[String], base: &Path) -> Vec<PathBuf> {
    values
        .iter()
        .flat_map(|pattern| {
            let path = PathBuf::from(resolve_identity_path(pattern, base));
            if !path.to_string_lossy().contains('*') && !path.to_string_lossy().contains('?') {
                return if path.is_file() {
                    vec![path]
                } else {
                    Vec::new()
                };
            }
            let Some(parent) = path.parent() else {
                return Vec::new();
            };
            let Some(file_pattern) = path.file_name().and_then(|name| name.to_str()) else {
                return Vec::new();
            };
            fs::read_dir(parent)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|candidate| {
                    candidate.is_file()
                        && candidate
                            .file_name()
                            .and_then(|name| name.to_str())
                            .map(|name| wildcard_match(file_pattern, name))
                            .unwrap_or(false)
                })
                .collect()
        })
        .collect()
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let mut matches = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    matches[0][0] = true;
    for index in 1..=pattern.len() {
        if pattern[index - 1] == '*' {
            matches[index][0] = matches[index - 1][0];
        }
        for value_index in 1..=value.len() {
            matches[index][value_index] = if pattern[index - 1] == '*' {
                matches[index - 1][value_index] || matches[index][value_index - 1]
            } else {
                matches[index - 1][value_index - 1]
                    && (pattern[index - 1] == '?' || pattern[index - 1] == value[value_index - 1])
            };
        }
    }
    matches[pattern.len()][value.len()]
}

fn resolve_identity_path(value: &str, base: &Path) -> String {
    resolve_identity_path_with_home(value, base, home_directory().as_deref())
}

fn home_directory() -> Option<PathBuf> {
    select_home_directory(
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
        cfg!(windows),
    )
}

fn select_home_directory(
    home: Option<PathBuf>,
    userprofile: Option<PathBuf>,
    windows: bool,
) -> Option<PathBuf> {
    if windows {
        userprofile.or(home)
    } else {
        home.or(userprofile)
    }
}

fn resolve_identity_path_with_home(value: &str, base: &Path, home: Option<&Path>) -> String {
    let rest = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"));
    if let (Some(rest), Some(home)) = (rest, home) {
        return join_home_path(home, rest);
    }
    let path = Path::new(value);
    if path.is_absolute() {
        value.to_owned()
    } else {
        base.join(path).to_string_lossy().into_owned()
    }
}

fn join_home_path(home: &Path, rest: &str) -> String {
    let home = home.to_string_lossy();
    let separator = if home.contains('\\') { '\\' } else { '/' };
    let home = home.trim_end_matches(['/', '\\']);
    let rest = rest.replace(['/', '\\'], &separator.to_string());
    format!("{home}{separator}{rest}")
}

fn validate_source_path(path: &str) -> Result<PathBuf, AppError> {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    let expanded = if let Some(rest) = rest {
        home_directory()
            .map(|home| PathBuf::from(join_home_path(&home, rest)))
            .unwrap_or_else(|| PathBuf::from(path))
    } else {
        PathBuf::from(path)
    };
    let path = expanded.as_path();
    if !path.is_file() {
        return Err(AppError::InvalidArgument(
            "SSH config file is unavailable".into(),
        ));
    }
    fs::canonicalize(path)
        .map_err(|_| AppError::InvalidArgument("SSH config file is unavailable".into()))
}

fn stable_profile_id(path: &Path, alias: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(alias.as_bytes());
    format!("openssh-config:{}", hex::encode(digest.finalize()))
}

fn stable_static_profile_id(name: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(name.as_bytes());
    format!("file-config:{}", hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use crate::error::AppError;
    use tempfile::tempdir;

    use super::{
        apply, parse_config, parse_config_line, preview, resolve_identity_path_with_home,
        select_home_directory, stable_static_profile_id, SshImportAction, SshImportSelection,
        SshImportSelectionItem, SshImportSource,
    };

    #[test]
    fn prefers_native_home_variable_for_each_platform() {
        assert_eq!(
            select_home_directory(
                Some(PathBuf::from("/msys/home/alice")),
                Some(PathBuf::from(r"C:\Users\alice")),
                true,
            ),
            Some(PathBuf::from(r"C:\Users\alice")),
        );
        assert_eq!(
            select_home_directory(
                Some(PathBuf::from("/home/alice")),
                Some(PathBuf::from(r"C:\Users\alice")),
                false,
            ),
            Some(PathBuf::from("/home/alice")),
        );
    }

    #[test]
    fn expands_tilde_identity_paths_from_explicit_home_directory() {
        let home = if cfg!(windows) {
            PathBuf::from(r"C:\Users\alice")
        } else {
            PathBuf::from("/home/alice")
        };
        let resolved =
            resolve_identity_path_with_home("~/.ssh/id_ed25519", Path::new("/tmp"), Some(&home));

        assert_eq!(
            PathBuf::from(resolved),
            home.join(".ssh").join("id_ed25519")
        );
    }

    #[test]
    fn preserves_posix_home_separators_on_every_host() {
        let resolved = resolve_identity_path_with_home(
            "~/.ssh/id_ed25519",
            Path::new("C:/tmp"),
            Some(Path::new("/home/alice")),
        );

        assert_eq!(resolved, "/home/alice/.ssh/id_ed25519");
    }

    #[cfg(windows)]
    #[test]
    fn expands_windows_tilde_identity_paths_from_explicit_home_directory() {
        let home = PathBuf::from(r"C:\Users\alice");
        let resolved = resolve_identity_path_with_home(
            r"~\.ssh\id_ed25519",
            Path::new(r"C:\tmp"),
            Some(&home),
        );

        assert_eq!(
            PathBuf::from(resolved),
            home.join(".ssh").join("id_ed25519")
        );
    }

    #[test]
    fn parses_quoted_include_and_identity_paths() {
        let directory = tempdir().unwrap();
        let include_directory = directory.path().join("parts with spaces");
        fs::create_dir(&include_directory).unwrap();
        let included = include_directory.join("ssh config");
        fs::write(
            &included,
            "Host quoted\n  HostName quoted.example # ignored\n  IdentityFile \"keys/id #1\"\n",
        )
        .unwrap();
        let config = directory.path().join("config");
        fs::write(
            &config,
            "Include \"parts with spaces/ssh config\" # trailing comment\n",
        )
        .unwrap();

        let profiles = parse_config(&config).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].host, "quoted.example");
        assert!(Path::new(&profiles[0].private_keys[0])
            .ends_with(Path::new("parts with spaces/keys/id #1")));

        let (_, values) =
            parse_config_line(r#"IdentityFile "C:\Users\alice\.ssh\id_ed25519" # comment"#)
                .unwrap();
        assert_eq!(values, vec![r#"C:\Users\alice\.ssh\id_ed25519"#]);
    }

    #[test]
    fn ignores_include_cycles_without_duplicate_profiles() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.conf");
        let second = directory.path().join("second.conf");
        fs::write(
            &first,
            "Include second.conf\nHost first\n  HostName first.example\n",
        )
        .unwrap();
        fs::write(
            &second,
            "Include first.conf\nHost second\n  HostName second.example\n",
        )
        .unwrap();

        let profiles = parse_config(&first).unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(profiles
            .iter()
            .any(|profile| profile.host == "first.example"));
        assert!(profiles
            .iter()
            .any(|profile| profile.host == "second.example"));
    }

    #[test]
    fn skips_malformed_config_lines_without_aborting_import() {
        let directory = tempdir().unwrap();
        let config = directory.path().join("config");
        fs::write(
            &config,
            "Host good\n  HostName good.example\n  IdentityFile \"unterminated\n  Port not-a-port\n  malformed-only-key\nHost next\n  HostName next.example\n",
        )
        .unwrap();

        let profiles = parse_config(&config).unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(profiles
            .iter()
            .any(|profile| profile.host == "good.example"));
        assert!(profiles
            .iter()
            .any(|profile| profile.host == "next.example"));
    }

    #[test]
    fn parses_supported_fields_and_include_without_commands() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("included.conf"),
            "Host included\n  HostName included.example\n  User bob\n",
        )
        .unwrap();
        let config = directory.path().join("config");
        fs::write(
            &config,
            "Include included.conf\nHost app\n  HostName app.example\n  Port 2200\n  IdentityFile ~/.ssh/id_ed25519\nHost *\n  User ignored\nHost !excluded wildcard*\n  HostName ignored.example\n",
        )
        .unwrap();
        let profiles = parse_config(&config).unwrap();
        assert_eq!(profiles.len(), 2);
        let app = profiles
            .iter()
            .find(|profile| profile.host == "app.example")
            .unwrap();
        assert_eq!(app.port, 2200);
        assert_eq!(app.private_keys.len(), 1);
        assert!(Path::new(&app.private_keys[0]).ends_with(Path::new(".ssh").join("id_ed25519")));
        let preview = preview(&SshImportSource {
            path: config.to_string_lossy().into_owned(),
            existing_profile_ids: vec![app.id.clone()],
        })
        .unwrap();
        assert!(preview
            .conflicts
            .iter()
            .any(|conflict| conflict.profile_id == app.id));
    }

    #[test]
    fn preserves_host_block_precedence_across_inline_include() {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("included.conf"),
            "Host app\n  HostName included.example\n  User included-user\n",
        )
        .unwrap();
        let config = directory.path().join("config");
        fs::write(
            &config,
            "Host app\n  HostName main.example\n  Include included.conf\n  User main-user\n",
        )
        .unwrap();

        let profiles = parse_config(&config).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].host, "main.example");
        assert_eq!(profiles[0].user.as_deref(), Some("included-user"));
    }

    #[test]
    fn applies_wildcard_defaults_and_all_literal_aliases_with_negations() {
        let directory = tempdir().unwrap();
        let config = directory.path().join("config");
        fs::write(
            &config,
            "Host *\n  User default-user\n  Port 2201\nHost app prod\n  HostName app.example\nHost !prod app*\n  User should-not-apply\n",
        )
        .unwrap();

        let profiles = parse_config(&config).unwrap();
        let app = profiles
            .iter()
            .find(|profile| profile.name.starts_with("app "))
            .unwrap();
        assert_eq!(app.host, "app.example");
        assert_eq!(app.user.as_deref(), Some("default-user"));
        assert_eq!(app.port, 2201);
        assert!(profiles
            .iter()
            .any(|profile| profile.name.starts_with("prod ")));
        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn applies_profiles_in_tabby_config_shape_without_secrets() {
        let directory = tempdir().unwrap();
        let config = directory.path().join("config");
        fs::write(&config, "version: 1\nprofiles: []\n").unwrap();
        fs::write(
            directory.path().join("ssh-config"),
            "Host app\n  HostName app.example\n  User alice\n  IdentityFile ~/.ssh/id_ed25519\n",
        )
        .unwrap();
        let imported = parse_config(&directory.path().join("ssh-config")).unwrap();
        let (yaml, profiles, skipped, failed) = apply(
            &config,
            SshImportSelection {
                path: directory
                    .path()
                    .join("ssh-config")
                    .to_string_lossy()
                    .into_owned(),
                expected_revision: None,
                selections: vec![SshImportSelectionItem {
                    profile_id: imported[0].id.clone(),
                    action: SshImportAction::Duplicate,
                }],
            },
            "version: 1\nprofiles: []\n",
            None,
        )
        .unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(skipped.is_empty());
        assert!(failed.is_empty());
        let root: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let profile = &root["profiles"][0];
        assert_eq!(profile["type"].as_str(), Some("ssh"));
        assert_eq!(profile["options"]["host"].as_str(), Some("app.example"));
        assert!(yaml.contains("privateKeys"));
        assert!(!yaml.contains("password"));
    }

    #[test]
    fn applies_skip_duplicate_overwrite_and_rejects_stale_preview() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("ssh-profiles.yaml");
        fs::write(
            &source,
            "- name: Office\n  options:\n    host: office.example\n    port: 2200\n    user: alice\n",
        )
        .unwrap();
        let profile_id = stable_static_profile_id("Office");
        let current_yaml = format!(
            "version: 1\nprofiles:\n  - id: '{profile_id}'\n    name: Existing Office\n    type: ssh\n    options:\n      host: old.example\n      port: 22\n"
        );
        let source_path = source.to_string_lossy().into_owned();

        let (_, imported, skipped, failed) = apply(
            directory.path().join("config.yaml").as_path(),
            SshImportSelection {
                path: source_path.clone(),
                expected_revision: None,
                selections: vec![SshImportSelectionItem {
                    profile_id: profile_id.clone(),
                    action: SshImportAction::Skip,
                }],
            },
            &current_yaml,
            None,
        )
        .unwrap();
        assert!(imported.is_empty());
        assert_eq!(skipped, vec![profile_id.clone()]);
        assert!(failed.is_empty());

        let (duplicate_yaml, imported, skipped, failed) = apply(
            directory.path().join("config.yaml").as_path(),
            SshImportSelection {
                path: source_path.clone(),
                expected_revision: None,
                selections: vec![SshImportSelectionItem {
                    profile_id: profile_id.clone(),
                    action: SshImportAction::Duplicate,
                }],
            },
            &current_yaml,
            None,
        )
        .unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].id, format!("{profile_id}:imported-2"));
        assert!(skipped.is_empty());
        assert!(failed.is_empty());
        let duplicate_root: serde_yaml::Value = serde_yaml::from_str(&duplicate_yaml).unwrap();
        assert_eq!(duplicate_root["profiles"].as_sequence().unwrap().len(), 2);
        assert!(duplicate_yaml.contains("Existing Office"));

        let (overwrite_yaml, imported, skipped, failed) = apply(
            directory.path().join("config.yaml").as_path(),
            SshImportSelection {
                path: source_path.clone(),
                expected_revision: None,
                selections: vec![SshImportSelectionItem {
                    profile_id: profile_id.clone(),
                    action: SshImportAction::Overwrite,
                }],
            },
            &current_yaml,
            None,
        )
        .unwrap();
        assert_eq!(imported[0].id, profile_id);
        assert!(skipped.is_empty());
        assert!(failed.is_empty());
        let overwrite_root: serde_yaml::Value = serde_yaml::from_str(&overwrite_yaml).unwrap();
        assert_eq!(overwrite_root["profiles"].as_sequence().unwrap().len(), 1);
        assert_eq!(
            overwrite_root["profiles"][0]["name"].as_str(),
            Some("Office")
        );

        let stale = apply(
            directory.path().join("config.yaml").as_path(),
            SshImportSelection {
                path: source_path,
                expected_revision: Some("stale-revision".into()),
                selections: vec![],
            },
            &current_yaml,
            Some("current-revision"),
        );
        assert!(matches!(stale, Err(AppError::Conflict(message)) if message.contains("changed")));
    }

    #[test]
    fn imports_static_tabby_ssh_profiles() {
        let directory = tempdir().unwrap();
        let config = directory.path().join("ssh-profiles.yaml");
        fs::write(
            &config,
            "- name: Office\n  options:\n    host: office.example\n    port: 2200\n    user: alice\n    privateKeys:\n      - /tmp/id_ed25519\n",
        )
        .unwrap();

        let preview = preview(&SshImportSource {
            path: config.to_string_lossy().into_owned(),
            existing_profile_ids: Vec::new(),
        })
        .unwrap();
        assert_eq!(preview.profiles.len(), 1);
        assert_eq!(preview.profiles[0].id, stable_static_profile_id("Office"));
        assert_eq!(preview.profiles[0].port, 2200);
        assert_eq!(preview.profiles[0].private_keys, vec!["/tmp/id_ed25519"]);
    }

    #[test]
    fn recognizes_only_private_key_file_names() {
        assert!(super::is_private_key_filename("id_ed25519"));
        assert!(super::is_private_key_filename("id_rsa2"));
        assert!(!super::is_private_key_filename("id_ed25519.pub"));
        assert!(super::is_private_key_filename("id_ed25519_sk"));
        assert!(!super::is_private_key_filename("id_ed25519-cert"));
    }
}
