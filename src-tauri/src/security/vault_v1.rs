use std::{collections::BTreeMap, sync::Arc};

use aes::Aes256;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use cbc::{
    cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit},
    Decryptor, Encryptor,
};
use pbkdf2::pbkdf2_hmac;
use rand::{rngs::OsRng, RngCore};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};
use sha2::Sha512;
use zeroize::Zeroize;

const PBKDF_ITERATIONS: u32 = 100_000;
const KEY_SALT_LENGTH: usize = 8;
const KEY_LENGTH: usize = 32;
const IV_LENGTH: usize = 16;
const AES_BLOCK_LENGTH: usize = 16;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredVault {
    pub version: u32,
    pub contents: String,
    pub key_salt: String,
    pub iv: String,
}

#[derive(Debug)]
pub struct VaultSecret {
    pub r#type: String,
    pub key: Map<String, Value>,
    pub value: SecretString,
}

#[derive(Debug)]
pub struct Vault {
    pub config: Value,
    pub secrets: Vec<VaultSecret>,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("vault is locked")]
    Locked,
    #[error("unsupported vault format version {0}")]
    UnsupportedVersion(u32),
    #[error("vault key salt is not valid hexadecimal")]
    InvalidKeySaltEncoding,
    #[error("vault key salt has an invalid length")]
    InvalidKeySaltLength,
    #[error("vault IV is not valid hexadecimal")]
    InvalidIvEncoding,
    #[error("vault IV has an invalid length")]
    InvalidIvLength,
    #[error("vault ciphertext is not valid base64")]
    InvalidCiphertextEncoding,
    #[error("vault ciphertext has an invalid length")]
    InvalidCiphertextLength,
    #[error("vault passphrase or ciphertext is invalid")]
    DecryptionFailed,
    #[error("vault plaintext has an invalid JSON structure")]
    InvalidPlaintext,
    #[error("vault secret has an invalid structure")]
    InvalidSecret,
    #[error("failed to serialize vault contents")]
    SerializationFailed,
    #[error("failed to obtain secure random bytes")]
    RandomFailed,
}

pub trait VaultCodec: Send + Sync {
    fn version(&self) -> u32;

    fn decrypt(&self, stored: &StoredVault, passphrase: &SecretString)
        -> Result<Vault, VaultError>;

    fn encrypt(&self, vault: &Vault, passphrase: &SecretString) -> Result<StoredVault, VaultError>;
}

pub struct VaultCodecs {
    codecs: BTreeMap<u32, Arc<dyn VaultCodec>>,
}

impl Default for VaultCodecs {
    fn default() -> Self {
        let mut codecs: BTreeMap<u32, Arc<dyn VaultCodec>> = BTreeMap::new();
        codecs.insert(1, Arc::new(VaultV1));
        Self { codecs }
    }
}

impl VaultCodecs {
    pub fn decrypt(
        &self,
        stored: &StoredVault,
        passphrase: &SecretString,
    ) -> Result<Vault, VaultError> {
        let codec = self
            .codecs
            .get(&stored.version)
            .ok_or(VaultError::UnsupportedVersion(stored.version))?;
        codec.decrypt(stored, passphrase)
    }

    pub fn encrypt(
        &self,
        version: u32,
        vault: &Vault,
        passphrase: &SecretString,
    ) -> Result<StoredVault, VaultError> {
        let codec = self
            .codecs
            .get(&version)
            .ok_or(VaultError::UnsupportedVersion(version))?;
        codec.encrypt(vault, passphrase)
    }
}

pub struct VaultV1;

impl VaultCodec for VaultV1 {
    fn version(&self) -> u32 {
        1
    }

    fn decrypt(
        &self,
        stored: &StoredVault,
        passphrase: &SecretString,
    ) -> Result<Vault, VaultError> {
        if stored.version != self.version() {
            return Err(VaultError::UnsupportedVersion(stored.version));
        }

        let salt = decode_fixed_hex::<KEY_SALT_LENGTH>(
            &stored.key_salt,
            VaultError::InvalidKeySaltEncoding,
            VaultError::InvalidKeySaltLength,
        )?;
        let iv = decode_fixed_hex::<IV_LENGTH>(
            &stored.iv,
            VaultError::InvalidIvEncoding,
            VaultError::InvalidIvLength,
        )?;
        let ciphertext = BASE64_STANDARD
            .decode(&stored.contents)
            .map_err(|_| VaultError::InvalidCiphertextEncoding)?;
        if ciphertext.is_empty() || ciphertext.len() % AES_BLOCK_LENGTH != 0 {
            return Err(VaultError::InvalidCiphertextLength);
        }

        let mut key = derive_key(passphrase, &salt);
        let plaintext = Decryptor::<Aes256>::new((&key).into(), (&iv).into())
            .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext)
            .map_err(|_| VaultError::DecryptionFailed);
        key.zeroize();
        let mut plaintext = plaintext?;

        let wire = serde_json::from_slice::<WireVault>(&plaintext)
            .map_err(|_| VaultError::InvalidPlaintext);
        plaintext.zeroize();
        let wire = wire?;
        wire.try_into()
    }

    fn encrypt(&self, vault: &Vault, passphrase: &SecretString) -> Result<StoredVault, VaultError> {
        let mut salt = [0_u8; KEY_SALT_LENGTH];
        let mut iv = [0_u8; IV_LENGTH];
        OsRng
            .try_fill_bytes(&mut salt)
            .map_err(|_| VaultError::RandomFailed)?;
        OsRng
            .try_fill_bytes(&mut iv)
            .map_err(|_| VaultError::RandomFailed)?;
        encrypt_with_material(vault, passphrase, &salt, &iv)
    }
}

#[derive(serde::Deserialize)]
struct WireVault {
    #[serde(default)]
    config: Value,
    #[serde(default)]
    secrets: Vec<WireVaultSecret>,
}

#[derive(serde::Deserialize)]
struct WireVaultSecret {
    r#type: String,
    key: Map<String, Value>,
    value: String,
}

#[derive(serde::Serialize)]
struct WireVaultRef<'a> {
    config: &'a Value,
    secrets: Vec<WireVaultSecretRef<'a>>,
}

#[derive(serde::Serialize)]
struct WireVaultSecretRef<'a> {
    r#type: &'a str,
    key: &'a Map<String, Value>,
    value: &'a str,
}

impl TryFrom<WireVault> for Vault {
    type Error = VaultError;

    fn try_from(wire: WireVault) -> Result<Self, Self::Error> {
        let mut secrets = Vec::with_capacity(wire.secrets.len());
        for secret in wire.secrets {
            if secret.r#type.is_empty() || secret.r#type.chars().any(char::is_control) {
                return Err(VaultError::InvalidSecret);
            }
            secrets.push(VaultSecret {
                r#type: secret.r#type,
                key: secret.key,
                value: SecretString::new(secret.value),
            });
        }
        Ok(Self {
            config: wire.config,
            secrets,
        })
    }
}

fn encrypt_with_material(
    vault: &Vault,
    passphrase: &SecretString,
    salt: &[u8; KEY_SALT_LENGTH],
    iv: &[u8; IV_LENGTH],
) -> Result<StoredVault, VaultError> {
    let wire = WireVaultRef {
        config: &vault.config,
        secrets: vault
            .secrets
            .iter()
            .map(|secret| WireVaultSecretRef {
                r#type: &secret.r#type,
                key: &secret.key,
                value: secret.value.expose_secret(),
            })
            .collect(),
    };
    let mut plaintext = serde_json::to_vec(&wire).map_err(|_| VaultError::SerializationFailed)?;
    let mut key = derive_key(passphrase, salt);
    let ciphertext = Encryptor::<Aes256>::new((&key).into(), iv.into())
        .encrypt_padded_vec_mut::<Pkcs7>(&plaintext);
    key.zeroize();
    plaintext.zeroize();

    Ok(StoredVault {
        version: 1,
        contents: BASE64_STANDARD.encode(ciphertext),
        key_salt: hex::encode(salt),
        iv: hex::encode(iv),
    })
}

fn derive_key(passphrase: &SecretString, salt: &[u8]) -> [u8; KEY_LENGTH] {
    let mut key = [0_u8; KEY_LENGTH];
    pbkdf2_hmac::<Sha512>(
        passphrase.expose_secret().as_bytes(),
        salt,
        PBKDF_ITERATIONS,
        &mut key,
    );
    key
}

fn decode_fixed_hex<const N: usize>(
    value: &str,
    encoding_error: VaultError,
    length_error: VaultError,
) -> Result<[u8; N], VaultError> {
    let decoded = hex::decode(value).map_err(|_| encoding_error)?;
    decoded.try_into().map_err(|_| length_error)
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use secrecy::{ExposeSecret, SecretString};

    use super::{encrypt_with_material, StoredVault, VaultCodec, VaultCodecs, VaultError, VaultV1};

    const PASSPHRASE: &str = "correct horse";

    fn fixture() -> StoredVault {
        serde_json::from_str(include_str!("fixtures/tabby-v1.json")).unwrap()
    }

    #[test]
    fn decrypts_tabby_v1_fixture() {
        let vault = VaultV1
            .decrypt(&fixture(), &SecretString::new(PASSPHRASE.into()))
            .unwrap();
        assert_eq!(vault.secrets[0].r#type, "ssh:password");
        assert_eq!(
            vault.secrets[0].value.expose_secret(),
            "fixture-only-secret"
        );
        assert_eq!(vault.secrets[1].r#type, "file");
    }

    #[test]
    fn reproduces_the_node_compatible_test_vector() {
        let passphrase = SecretString::new(PASSPHRASE.into());
        let vault = VaultV1.decrypt(&fixture(), &passphrase).unwrap();
        let salt = [0, 1, 2, 3, 4, 5, 6, 7];
        let iv = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ];
        let stored = encrypt_with_material(&vault, &passphrase, &salt, &iv).unwrap();
        assert_eq!(stored, fixture());
    }

    #[test]
    fn rejects_wrong_passphrase_without_exposing_it() {
        let error = VaultV1
            .decrypt(&fixture(), &SecretString::new("incorrect".into()))
            .unwrap_err();
        assert!(matches!(error, VaultError::DecryptionFailed));
        assert!(!error.to_string().contains("incorrect"));
    }

    #[test]
    fn rejects_truncated_ciphertext() {
        let mut stored = fixture();
        let mut ciphertext = BASE64_STANDARD.decode(&stored.contents).unwrap();
        ciphertext.pop();
        stored.contents = BASE64_STANDARD.encode(ciphertext);
        assert!(matches!(
            VaultV1.decrypt(&stored, &SecretString::new(PASSPHRASE.into())),
            Err(VaultError::InvalidCiphertextLength)
        ));
    }

    #[test]
    fn rejects_invalid_material_and_unknown_versions() {
        let mut stored = fixture();
        stored.key_salt = "00".into();
        assert!(matches!(
            VaultV1.decrypt(&stored, &SecretString::new(PASSPHRASE.into())),
            Err(VaultError::InvalidKeySaltLength)
        ));

        let mut stored = fixture();
        stored.version = 2;
        assert!(matches!(
            VaultCodecs::default().decrypt(&stored, &SecretString::new(PASSPHRASE.into())),
            Err(VaultError::UnsupportedVersion(2))
        ));
    }
}
