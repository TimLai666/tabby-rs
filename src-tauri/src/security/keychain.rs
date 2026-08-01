use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::SecretString;

use crate::identity::CREDENTIAL_SERVICE;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialAddress {
    pub service: String,
    pub account: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialNamespace {
    TabbyRs,
    OriginalTabby,
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential address is invalid")]
    InvalidAddress,
    #[error("credential store is unavailable")]
    Unavailable,
}

pub trait CredentialStore: Send + Sync {
    fn get(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
    ) -> Result<Option<SecretString>, CredentialError>;

    fn put(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
        value: &str,
    ) -> Result<(), CredentialError>;

    fn delete(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
    ) -> Result<bool, CredentialError>;
}

#[derive(Default)]
pub struct NativeKeychain {
    access_lock: Mutex<()>,
}

impl CredentialStore for NativeKeychain {
    fn get(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
    ) -> Result<Option<SecretString>, CredentialError> {
        let _guard = self.lock();
        let entry = entry(namespace, address)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(SecretString::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialError::Unavailable),
        }
    }

    fn put(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
        value: &str,
    ) -> Result<(), CredentialError> {
        let _guard = self.lock();
        entry(namespace, address)?
            .set_password(value)
            .map_err(|_| CredentialError::Unavailable)
    }

    fn delete(
        &self,
        namespace: CredentialNamespace,
        address: &CredentialAddress,
    ) -> Result<bool, CredentialError> {
        let _guard = self.lock();
        match entry(namespace, address)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(CredentialError::Unavailable),
        }
    }
}

impl NativeKeychain {
    fn lock(&self) -> MutexGuard<'_, ()> {
        self.access_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Clone)]
pub struct CredentialState {
    store: Arc<dyn CredentialStore>,
}

impl Default for CredentialState {
    fn default() -> Self {
        Self {
            store: Arc::new(NativeKeychain::default()),
        }
    }
}

impl CredentialState {
    pub fn store(&self) -> Arc<dyn CredentialStore> {
        Arc::clone(&self.store)
    }

    #[cfg(test)]
    pub fn with_store(store: Arc<dyn CredentialStore>) -> Self {
        Self { store }
    }
}

fn entry(
    namespace: CredentialNamespace,
    address: &CredentialAddress,
) -> Result<keyring::Entry, CredentialError> {
    validate_address(address)?;
    let service = scoped_service(namespace, &address.service)?;
    keyring::Entry::new(&service, &address.account).map_err(|_| CredentialError::Unavailable)
}

fn scoped_service(
    namespace: CredentialNamespace,
    service: &str,
) -> Result<String, CredentialError> {
    let service = match namespace {
        CredentialNamespace::TabbyRs => format!("{CREDENTIAL_SERVICE}:{service}"),
        CredentialNamespace::OriginalTabby => service.to_owned(),
    };
    if service.len() > 512 {
        return Err(CredentialError::InvalidAddress);
    }
    Ok(service)
}

fn validate_address(address: &CredentialAddress) -> Result<(), CredentialError> {
    if address.service.is_empty()
        || address.account.is_empty()
        || address.service.len() > 480
        || address.account.len() > 512
        || address.service.chars().any(char::is_control)
        || address.account.chars().any(char::is_control)
    {
        return Err(CredentialError::InvalidAddress);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{scoped_service, validate_address, CredentialAddress, CredentialNamespace};

    #[test]
    fn keeps_original_and_tabby_rs_namespaces_distinct() {
        assert_eq!(
            scoped_service(CredentialNamespace::OriginalTabby, "ssh@example.test").unwrap(),
            "ssh@example.test"
        );
        assert_eq!(
            scoped_service(CredentialNamespace::TabbyRs, "ssh@example.test").unwrap(),
            "tabby-rs:ssh@example.test"
        );
    }

    #[test]
    fn rejects_empty_or_control_character_addresses() {
        assert!(validate_address(&CredentialAddress {
            service: String::new(),
            account: "alice".into(),
        })
        .is_err());
        assert!(validate_address(&CredentialAddress {
            service: "ssh@example.test".into(),
            account: "ali\nce".into(),
        })
        .is_err());
    }
}
