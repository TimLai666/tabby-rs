use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use secrecy::{ExposeSecret, SecretString};
use serde_yaml::{Mapping, Value as YamlValue};
use sha2::{Digest, Sha256};

use crate::{
    error::AppError,
    storage::{
        atomic_file::read_required_regular_file, migration::detect_import_plans,
        paths::StoragePaths,
    },
};

use super::{
    CredentialAddress, CredentialNamespace, CredentialStore, SecretState, StoredVault, VaultCodecs,
    VaultMutationResult, VaultSnapshot, VaultSnapshotSecret,
};

const MAX_SOURCE_CONFIG_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_REMEMBER_SECONDS: u64 = 300;
const MAX_REMEMBER_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportSource {
    pub source_data_dir: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportItem {
    pub id: String,
    pub source: SecretImportItemSource,
    pub kind: String,
    pub label: String,
    pub requires_passphrase: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SecretImportItemSource {
    Vault,
    Keychain,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportPlan {
    pub source_data_dir: String,
    pub items: Vec<SecretImportItem>,
    pub requires_authorization: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportSelection {
    pub source_data_dir: String,
    pub authorized: bool,
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub source_vault_passphrase: Option<String>,
    #[serde(default = "default_remember_seconds")]
    pub remember_for_seconds: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportFailure {
    pub id: String,
    pub public_error: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretImportReport {
    pub imported: Vec<String>,
    pub requires_reentry: Vec<String>,
    pub failed: Vec<SecretImportFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_mutation: Option<VaultMutationResult>,
}

#[derive(Debug, Clone)]
struct PlannedKeychainItem {
    address: CredentialAddress,
}

#[derive(Debug, Clone)]
struct InternalPlan {
    public: SecretImportPlan,
    source_vault: Option<StoredVault>,
    keychain_items: BTreeMap<String, PlannedKeychainItem>,
}

pub fn plan_secret_import(
    paths: &StoragePaths,
    source: &SecretImportSource,
) -> Result<SecretImportPlan, AppError> {
    Ok(build_plan(paths, source)?.public)
}

pub fn execute_secret_import(
    paths: &StoragePaths,
    selection: SecretImportSelection,
    secrets: &SecretState,
    credentials: Arc<dyn CredentialStore>,
) -> Result<SecretImportReport, AppError> {
    if !selection.authorized {
        return Err(AppError::PermissionDenied(
            "secret import requires explicit authorization".into(),
        ));
    }
    if selection.remember_for_seconds == 0 || selection.remember_for_seconds > MAX_REMEMBER_SECONDS
    {
        return Err(AppError::InvalidArgument(
            "secret import remember timeout is invalid".into(),
        ));
    }

    let plan = build_plan(
        paths,
        &SecretImportSource {
            source_data_dir: selection.source_data_dir.clone(),
        },
    )?;
    let selected = selection.item_ids.into_iter().collect::<BTreeSet<_>>();
    if selected.is_empty() {
        return Err(AppError::InvalidArgument(
            "at least one secret import item must be selected".into(),
        ));
    }
    let known = plan
        .public
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    if selected.iter().any(|id| !known.contains(id.as_str())) {
        return Err(AppError::InvalidArgument(
            "secret import selection does not match the current plan".into(),
        ));
    }

    let mut imported = Vec::new();
    let mut requires_reentry = Vec::new();
    let mut failed = Vec::new();
    let mut vault_mutation = None;

    if let Some(vault_item) =
        plan.public.items.iter().find(|item| {
            item.source == SecretImportItemSource::Vault && selected.contains(&item.id)
        })
    {
        match import_vault(
            plan.source_vault
                .ok_or_else(|| AppError::InvalidData("source Vault is unavailable".into()))?,
            selection.source_vault_passphrase,
            selection.remember_for_seconds,
            secrets,
        ) {
            Ok(mutation) => {
                imported.push(vault_item.id.clone());
                vault_mutation = Some(mutation);
            }
            Err(error) => {
                requires_reentry.push(vault_item.id.clone());
                failed.push(SecretImportFailure {
                    id: vault_item.id.clone(),
                    public_error: public_import_error(&error),
                });
            }
        }
    }

    for id in selected {
        let Some(item) = plan.keychain_items.get(&id) else {
            continue;
        };
        match credentials.get(CredentialNamespace::OriginalTabby, &item.address) {
            Ok(Some(value)) => {
                match credentials.put(
                    CredentialNamespace::TabbyRs,
                    &item.address,
                    value.expose_secret(),
                ) {
                    Ok(()) => imported.push(id),
                    Err(_) => failed.push(SecretImportFailure {
                        id,
                        public_error:
                            "Could not write this credential to the Tabby RS keychain namespace."
                                .into(),
                    }),
                }
            }
            Ok(None) => requires_reentry.push(id),
            Err(_) => failed.push(SecretImportFailure {
                id,
                public_error:
                    "Could not read this credential from the original keychain namespace.".into(),
            }),
        }
    }

    Ok(SecretImportReport {
        imported,
        requires_reentry,
        failed,
        vault_mutation,
    })
}

fn build_plan(paths: &StoragePaths, source: &SecretImportSource) -> Result<InternalPlan, AppError> {
    let source_dir = validate_source(paths, &source.source_data_dir)?;
    build_plan_from_source(source_dir)
}

fn build_plan_from_source(source_dir: PathBuf) -> Result<InternalPlan, AppError> {
    let config_bytes = read_required_regular_file(&source_dir.join("config.yaml"))?;
    if config_bytes.len() > MAX_SOURCE_CONFIG_BYTES {
        return Err(AppError::InvalidData(
            "source config.yaml is too large".into(),
        ));
    }
    let root: YamlValue = serde_yaml::from_slice(&config_bytes)?;
    let source_text = source_dir.to_string_lossy().into_owned();

    let mut items = Vec::new();
    let source_vault = extract_vault(&root);
    if source_vault.is_some() {
        items.push(SecretImportItem {
            id: stable_item_id("vault", &source_text, "vault", "v1"),
            source: SecretImportItemSource::Vault,
            kind: "vault".into(),
            label: "Original Tabby Vault".into(),
            requires_passphrase: true,
        });
    }

    let mut keychain_items = BTreeMap::new();
    for (index, profile) in extract_ssh_profiles(&root).into_iter().enumerate() {
        if !profile.user.is_empty() {
            let service = match profile.port {
                Some(port) => format!("ssh@{}:{port}", profile.host),
                None => format!("ssh@{}", profile.host),
            };
            let address = CredentialAddress {
                service,
                account: profile.user.clone(),
            };
            let id = stable_item_id("keychain", &source_text, &address.service, &address.account);
            items.push(SecretImportItem {
                id: id.clone(),
                source: SecretImportItemSource::Keychain,
                kind: "sshPassword".into(),
                label: format!("SSH password {}", index + 1),
                requires_passphrase: false,
            });
            keychain_items.insert(id, PlannedKeychainItem { address });
        }

        for (key_index, private_key) in profile.private_keys.into_iter().enumerate() {
            if private_key.is_empty() {
                continue;
            }
            let address = CredentialAddress {
                service: format!("ssh-private-key:{private_key}"),
                account: "user".into(),
            };
            let id = stable_item_id("keychain", &source_text, &address.service, &address.account);
            items.push(SecretImportItem {
                id: id.clone(),
                source: SecretImportItemSource::Keychain,
                kind: "privateKeyPassphrase".into(),
                label: format!("Private-key passphrase {}.{}", index + 1, key_index + 1),
                requires_passphrase: false,
            });
            keychain_items.insert(id, PlannedKeychainItem { address });
        }
    }

    items.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(InternalPlan {
        public: SecretImportPlan {
            source_data_dir: source_text,
            requires_authorization: !items.is_empty(),
            items,
        },
        source_vault,
        keychain_items,
    })
}

fn validate_source(paths: &StoragePaths, source: &str) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(source)
        .map_err(|_| AppError::InvalidArgument("secret import source is unavailable".into()))?;
    let allowed = detect_import_plans(paths)?
        .iter()
        .any(|plan| Path::new(&plan.source_data_dir) == canonical.as_path());
    if !allowed {
        return Err(AppError::PermissionDenied(
            "secret import source was not detected by config migration".into(),
        ));
    }
    Ok(canonical)
}

fn extract_vault(root: &YamlValue) -> Option<StoredVault> {
    let mapping = root.as_mapping()?;
    serde_yaml::from_value(mapping_get(mapping, "vault")?.clone()).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SshProfileCandidate {
    host: String,
    port: Option<u16>,
    user: String,
    private_keys: Vec<String>,
}

fn extract_ssh_profiles(root: &YamlValue) -> Vec<SshProfileCandidate> {
    let mut profiles = BTreeSet::new();
    visit_yaml(root, &mut |mapping| {
        if let Some(profile) = profile_from_mapping(mapping) {
            profiles.insert(profile);
        }
    });
    profiles.into_iter().collect()
}

fn profile_from_mapping(mapping: &Mapping) -> Option<SshProfileCandidate> {
    let candidate = mapping_get(mapping, "options")
        .and_then(YamlValue::as_mapping)
        .unwrap_or(mapping);
    let host = mapping_string(candidate, "host")?;
    if host.is_empty() {
        return None;
    }
    let private_keys = mapping_get(candidate, "privateKeys")
        .and_then(YamlValue::as_sequence)
        .map(|items| {
            items
                .iter()
                .filter_map(YamlValue::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(SshProfileCandidate {
        host,
        port: mapping_u16(candidate, "port"),
        user: mapping_string(candidate, "user").unwrap_or_default(),
        private_keys,
    })
}

fn visit_yaml(value: &YamlValue, visitor: &mut impl FnMut(&Mapping)) {
    match value {
        YamlValue::Mapping(mapping) => {
            visitor(mapping);
            for child in mapping.values() {
                visit_yaml(child, visitor);
            }
        }
        YamlValue::Sequence(sequence) => {
            for child in sequence {
                visit_yaml(child, visitor);
            }
        }
        _ => {}
    }
}

fn mapping_get<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(&YamlValue::String(key.into()))
}

fn mapping_string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping_get(mapping, key)
        .and_then(YamlValue::as_str)
        .map(str::to_owned)
}

fn mapping_u16(mapping: &Mapping, key: &str) -> Option<u16> {
    mapping_get(mapping, key)
        .and_then(YamlValue::as_u64)
        .and_then(|port| u16::try_from(port).ok())
}

fn stable_item_id(source: &str, root: &str, service: &str, account: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(source.as_bytes());
    digest.update([0]);
    digest.update(root.as_bytes());
    digest.update([0]);
    digest.update(service.as_bytes());
    digest.update([0]);
    digest.update(account.as_bytes());
    format!("{source}:{}", hex::encode(digest.finalize()))
}

fn import_vault(
    stored: StoredVault,
    passphrase: Option<String>,
    remember_for_seconds: u64,
    secrets: &SecretState,
) -> Result<VaultMutationResult, AppError> {
    let passphrase = passphrase
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::PermissionDenied("source Vault passphrase is required".into()))?;
    let passphrase = SecretString::new(passphrase);
    let vault = VaultCodecs::default().decrypt(&stored, &passphrase)?;
    let snapshot = VaultSnapshot {
        config: vault.config,
        secrets: vault
            .secrets
            .into_iter()
            .map(|secret| VaultSnapshotSecret {
                r#type: secret.r#type,
                key: secret.key,
                value: secret.value.expose_secret().to_owned(),
            })
            .collect(),
    };
    Ok(secrets.replace(
        snapshot,
        passphrase,
        Duration::from_secs(remember_for_seconds),
    )?)
}

fn public_import_error(error: &AppError) -> String {
    match error {
        AppError::PermissionDenied(_) => {
            "The Vault could not be unlocked with the supplied authorization.".into()
        }
        AppError::InvalidData(_) => "The original Vault is invalid or unsupported.".into(),
        _ => "The Vault could not be imported.".into(),
    }
}

const fn default_remember_seconds() -> u64 {
    DEFAULT_REMEMBER_SECONDS
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_yaml::Value as YamlValue;
    use tempfile::tempdir;

    use super::{build_plan_from_source, extract_ssh_profiles, stable_item_id};

    #[test]
    fn extracts_keychain_candidates_without_secret_values() {
        let root: YamlValue = serde_yaml::from_str(
            "profiles:\n  - type: ssh\n    options:\n      host: example.test\n      port: 2222\n      user: alice\n      privateKeys: [key-id]\n",
        )
        .unwrap();
        let profiles = extract_ssh_profiles(&root);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].host, "example.test");
        assert_eq!(profiles[0].private_keys, vec!["key-id"]);
    }

    #[test]
    fn plan_contains_only_metadata_and_opaque_ids() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("config.yaml"),
            "profiles:\n  - type: ssh\n    options:\n      host: example.test\n      user: alice\n",
        )
        .unwrap();
        let plan = build_plan_from_source(source).unwrap().public;
        assert_eq!(plan.items.len(), 1);
        let serialized = serde_json::to_string(&plan).unwrap();
        assert!(!serialized.contains("example.test"));
        assert!(!serialized.contains("alice"));
    }

    #[test]
    fn item_ids_are_stable_and_do_not_embed_addresses() {
        let id = stable_item_id("keychain", "/source", "ssh@example.test", "alice");
        assert_eq!(
            id,
            stable_item_id("keychain", "/source", "ssh@example.test", "alice")
        );
        assert!(!id.contains("example.test"));
        assert!(!id.contains("alice"));
    }
}
