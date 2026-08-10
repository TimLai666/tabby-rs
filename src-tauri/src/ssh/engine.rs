use std::{collections::BTreeMap, sync::Arc};

#[async_trait::async_trait]
pub trait HostKeyVerifier: Send + Sync {
    async fn verify(
        &self,
        host: &str,
        port: u16,
        algorithm: &str,
        fingerprint_sha256: &str,
    ) -> Result<bool, crate::ssh::SshError>;
}

#[async_trait::async_trait]
pub trait SshAuthenticator: Send + Sync {
    async fn authenticate(
        &self,
        username: &str,
        methods: &[crate::ssh::AuthMethodRef],
    ) -> Result<bool, crate::ssh::SshError>;
}

#[derive(Debug, Clone)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct ShellChannelRequest {
    pub term: String,
    pub columns: u32,
    pub rows: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub environment: BTreeMap<String, String>,
}

#[async_trait::async_trait]
pub trait SshEngine: Send + Sync {
    async fn connect(
        &self,
        target: SshTarget,
        verifier: Arc<dyn HostKeyVerifier>,
        authenticator: Arc<dyn SshAuthenticator>,
    ) -> Result<Box<dyn SshConnection>, crate::ssh::SshError>;
}

#[async_trait::async_trait]
pub trait SshConnection: Send + Sync {
    async fn open_shell(
        &self,
        request: ShellChannelRequest,
    ) -> Result<Box<dyn SshChannel>, crate::ssh::SshError>;

    async fn disconnect(&self) -> Result<(), crate::ssh::SshError>;
}

#[async_trait::async_trait]
pub trait SshChannel: Send + Sync {
    async fn write(&self, data: &[u8]) -> Result<(), crate::ssh::SshError>;
    async fn resize(
        &self,
        columns: u32,
        rows: u32,
        pixel_width: u32,
        pixel_height: u32,
    ) -> Result<(), crate::ssh::SshError>;
    async fn close(&self) -> Result<(), crate::ssh::SshError>;
}
