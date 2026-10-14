use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

use russh::{
    client::{self, AuthResult},
    keys::{decode_secret_key, PrivateKeyWithHashAlg},
    Disconnect,
};
use russh_sftp::client::SftpSession;
use tokio::{fs, time::timeout};

use super::{AcceptAnyHostKey, OpenSshFixture};
use crate::ssh::sftp::{SftpManager, SftpOverwritePolicy};

#[tokio::test]
#[ignore = "requires an isolated local OpenSSH server"]
async fn overwrites_existing_openssh_files_and_restores_editor_permissions() {
    let mut fixture = OpenSshFixture::start()
        .await
        .expect("start isolated OpenSSH fixture");
    let data = tempfile::tempdir().expect("create test-owned upload directory");
    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let mut ssh = timeout(
        Duration::from_secs(20),
        client::connect(config, ("127.0.0.1", fixture.port), AcceptAnyHostKey),
    )
    .await
    .expect("SSH connection timed out")
    .expect("connect to isolated SSH server");
    let key_text = fs::read_to_string(&fixture.private_key)
        .await
        .expect("read test-owned client key");
    let key = decode_secret_key(&key_text, None).expect("decode fixture key");
    let auth = ssh
        .authenticate_publickey(
            &fixture.username,
            PrivateKeyWithHashAlg::new(Arc::new(key), None),
        )
        .await
        .expect("authenticate isolated SSH session");
    assert!(matches!(auth, AuthResult::Success));
    let channel = ssh.channel_open_session().await.expect("open SFTP channel");
    channel
        .request_subsystem(true, "sftp")
        .await
        .expect("request SFTP subsystem");
    let session = SftpSession::new(channel.into_stream())
        .await
        .expect("initialize actual OpenSSH SFTP session");
    let mut manager = SftpManager::new(session);
    let mut results = Vec::new();
    for (name, size, payload) in [
        ("binary.bin", Some(5), &[0, 255, 1, 128, 0][..]),
        ("zero.bin", Some(0), &[][..]),
        ("unknown-\u{4e2d}\u{6587}.bin", None, &[4, 0, 254][..]),
    ] {
        let target = data.path().join(name);
        fs::write(&target, b"original destination")
            .await
            .expect("initialize test-owned destination");
        fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
            .await
            .expect("set test-owned original permissions");
        let path = target.to_str().expect("fixture path is UTF-8");
        let original_mode = manager.stat(path, false).await.unwrap().mode & 0o777;
        let upload = manager
            .open_upload(path, size, SftpOverwritePolicy::Overwrite)
            .await
            .expect("open overwrite of existing file");
        manager.write(&upload.id, payload).await.unwrap();
        assert_eq!(
            fs::read(&target).await.unwrap(),
            b"original destination",
            "uploading a stage must leave the destination intact"
        );
        // Capture commit errors, then clean the session/server before asserting.
        let result = manager.close(&upload.id).await;
        if result.is_ok() {
            // Edit locally restores the listed original mode after its upload.
            manager.chmod(path, original_mode).await.unwrap();
        }
        let bytes = fs::read(&target).await.unwrap();
        let mode = fs::metadata(&target).await.unwrap().permissions().mode() & 0o777;
        results.push((name, result, bytes, mode, payload.to_vec()));
    }
    let mut entries = fs::read_dir(data.path()).await.unwrap();
    let mut stages = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(".tabby-upload-")
        {
            stages.push(entry.file_name());
        }
    }
    timeout(Duration::from_secs(5), manager.shutdown())
        .await
        .expect("SFTP shutdown timed out");
    ssh.disconnect(Disconnect::ByApplication, "overwrite fixture complete", "")
        .await
        .expect("disconnect fixture SSH session");
    if let Some(mut child) = fixture.child.take() {
        child.kill().await.expect("stop test-owned SSH server");
        child.wait().await.expect("reap test-owned SSH server");
    }
    assert!(stages.is_empty(), "owned upload stages remain: {stages:?}");
    for (name, result, bytes, mode, expected) in results {
        assert_eq!(
            result
                .unwrap_or_else(|error| panic!(
                    "existing-file overwrite failed for {name}: {error}"
                ))
                .state,
            "completed"
        );
        assert_eq!(bytes, expected, "exact uploaded bytes for {name}");
        assert_eq!(mode, 0o600, "Edit locally mode restoration for {name}");
    }
}
