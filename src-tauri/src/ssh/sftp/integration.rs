use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::Arc,
    time::Duration,
};

use russh::{
    client::{self, AuthResult, Handler},
    keys::{decode_secret_key, PrivateKeyWithHashAlg},
    ChannelMsg, Disconnect,
};
use tempfile::TempDir;
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
    process::{Child, Command},
    time::{sleep, timeout},
};

use crate::ssh::engine::{
    HostKeyVerifier, PrivateKeyMaterial, ShellChannelRequest, SshAuthContext, SshAuthenticator,
    SshEngine, SshHostKey, SshTarget,
};

use super::{manager::SftpManager, SftpOverwritePolicy};

#[derive(Default)]
struct AcceptAnyHostKey;

impl Handler for AcceptAnyHostKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

struct OpenSshFixture {
    _directory: TempDir,
    child: Option<Child>,
    port: u16,
    username: String,
    private_key: PathBuf,
}

impl Drop for OpenSshFixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

impl OpenSshFixture {
    async fn start() -> Result<Self, String> {
        if std::env::var("TABBY_RS_SSH_INTEGRATION").as_deref() != Ok("1") {
            return Err(
                "set TABBY_RS_SSH_INTEGRATION=1 to run the OpenSSH integration fixture".into(),
            );
        }

        let sshd = std::env::var_os("TABBY_RS_SSHD").unwrap_or_else(|| "/usr/sbin/sshd".into());
        let keygen =
            std::env::var_os("TABBY_RS_SSH_KEYGEN").unwrap_or_else(|| "/usr/bin/ssh-keygen".into());
        let username = command_output("id", ["-un"])?;
        let username = username.trim().to_owned();
        if username.is_empty() || username.chars().any(char::is_whitespace) {
            return Err("current account name is empty or contains whitespace".into());
        }

        ProcessCommand::new(&sshd)
            .arg("-V")
            .output()
            .map_err(|error| format!("cannot execute sshd: {error}"))?;

        let directory = tempfile::tempdir().map_err(|error| format!("tempdir: {error}"))?;
        let host_key = directory.path().join("host_key");
        let private_key = directory.path().join("client_key");
        generate_key(&keygen, &host_key)?;
        generate_key(&keygen, &private_key)?;

        let public_key = private_key.with_extension("pub");
        let authorized_keys = directory.path().join("authorized_keys");
        fs::copy(&public_key, &authorized_keys)
            .map_err(|error| format!("copy authorized_keys: {error}"))?;

        let port = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("reserve TCP port: {error}"))?
            .local_addr()
            .map_err(|error| format!("read reserved TCP port: {error}"))?
            .port();
        let config = directory.path().join("sshd_config");
        fs::write(
            &config,
            format!(
                "Port {port}\n\
                 ListenAddress 127.0.0.1\n\
                 HostKey {}\n\
                 AuthorizedKeysFile {}\n\
                 PubkeyAuthentication yes\n\
                 PasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\n\
                 UsePAM no\n\
                 StrictModes no\n\
                 PermitRootLogin no\n\
                 Subsystem sftp internal-sftp\n\
                 AllowUsers {username}\n\
                 PidFile {}\n\
                 PrintMotd no\n\
                 LogLevel ERROR\n",
                host_key.display(),
                authorized_keys.display(),
                directory.path().join("sshd.pid").display(),
            ),
        )
        .map_err(|error| format!("write sshd config: {error}"))?;

        let mut child = Command::new(&sshd)
            .args(["-D", "-e", "-f"])
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start sshd: {error}"))?;

        let mut ready = false;
        for _ in 0..100 {
            if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                ready = true;
                break;
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("check sshd: {error}"))?
            {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr).await;
                }
                return Err(format!(
                    "sshd exited before becoming ready: {status}: {}",
                    stderr.trim()
                ));
            }
            sleep(Duration::from_millis(50)).await;
        }
        if !ready {
            let _ = child.start_kill();
            return Err("sshd did not become ready within five seconds".into());
        }

        Ok(Self {
            _directory: directory,
            child: Some(child),
            port,
            username,
            private_key,
        })
    }
}

fn command_output<const N: usize>(program: &str, args: [&str; N]) -> Result<String, String> {
    let output = ProcessCommand::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("execute {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{program} output is not UTF-8: {error}"))
}

fn generate_key(program: &std::ffi::OsStr, path: &std::path::Path) -> Result<(), String> {
    let output = ProcessCommand::new(program)
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path)
        .output()
        .map_err(|error| format!("generate SSH key: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

struct FixtureHostKeyVerifier {
    accept: bool,
}

#[async_trait::async_trait]
impl HostKeyVerifier for FixtureHostKeyVerifier {
    async fn verify(
        &self,
        _host: &str,
        _port: u16,
        _key: &SshHostKey,
    ) -> Result<bool, crate::ssh::SshError> {
        Ok(self.accept)
    }
}

struct FixtureAuthenticator {
    private_key: Vec<u8>,
}

#[async_trait::async_trait]
impl SshAuthenticator for FixtureAuthenticator {
    async fn authenticate(
        &self,
        context: &mut dyn SshAuthContext,
        username: &str,
        _methods: &[crate::ssh::AuthMethodRef],
    ) -> Result<bool, crate::ssh::SshError> {
        context
            .authenticate_private_key(
                username,
                PrivateKeyMaterial {
                    openssh: self.private_key.clone(),
                    passphrase: None,
                },
            )
            .await
    }
}

#[tokio::test]
#[ignore = "requires a local OpenSSH server; run yarn test:ssh-integration"]
async fn runs_real_ssh_shell_and_sftp_lifecycle() {
    let fixture = OpenSshFixture::start()
        .await
        .unwrap_or_else(|error| panic!("OpenSSH fixture failed: {error}"));

    let engine = crate::ssh::engine::RusshEngine::new(
        client::Config {
            inactivity_timeout: Some(Duration::from_secs(30)),
            ..Default::default()
        },
        Duration::from_secs(20),
    );
    let rejected = engine
        .connect(
            SshTarget {
                host: "127.0.0.1".into(),
                port: fixture.port,
                username: fixture.username.clone(),
            },
            Arc::new(FixtureHostKeyVerifier { accept: false }),
            Arc::new(FixtureAuthenticator {
                private_key: fs::read(&fixture.private_key).expect("read rejected client key"),
            }),
        )
        .await;
    assert!(matches!(
        rejected,
        Err(crate::ssh::SshError::HostKeyRejected)
    ));

    let engine_connection = engine
        .connect(
            SshTarget {
                host: "127.0.0.1".into(),
                port: fixture.port,
                username: fixture.username.clone(),
            },
            Arc::new(FixtureHostKeyVerifier { accept: true }),
            Arc::new(FixtureAuthenticator {
                private_key: fs::read(&fixture.private_key).expect("read engine client key"),
            }),
        )
        .await
        .expect("engine SSH connection failed");
    let engine_shell = engine_connection
        .open_shell(ShellChannelRequest {
            term: "xterm".into(),
            columns: 80,
            rows: 24,
            pixel_width: 0,
            pixel_height: 0,
            environment: Default::default(),
        })
        .await
        .expect("engine shell channel failed");
    engine_shell
        .write(b"printf 'tabby-rs-engine\\n'; exit\\n")
        .await
        .expect("engine shell write failed");
    let mut engine_output = Vec::new();
    for _ in 0..16 {
        let message = timeout(Duration::from_secs(10), engine_shell.read())
            .await
            .expect("engine shell response timed out")
            .expect("engine shell read failed");
        let Some(message) = message else { break };
        match message {
            crate::ssh::engine::SshChannelMessage::Data(data)
            | crate::ssh::engine::SshChannelMessage::ExtendedData { data, .. } => {
                engine_output.extend_from_slice(&data)
            }
            crate::ssh::engine::SshChannelMessage::Close => break,
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&engine_output).contains("tabby-rs-engine"));
    engine_connection
        .disconnect()
        .await
        .expect("engine disconnect failed");

    let config = Arc::new(client::Config {
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    });
    let mut handle = timeout(
        Duration::from_secs(20),
        client::connect(config, ("127.0.0.1", fixture.port), AcceptAnyHostKey),
    )
    .await
    .expect("SSH connection timed out")
    .expect("SSH connection failed");

    let key_text = fs::read_to_string(&fixture.private_key).expect("read client key");
    let key = decode_secret_key(&key_text, None).expect("decode client key");
    let auth = handle
        .authenticate_publickey(
            &fixture.username,
            PrivateKeyWithHashAlg::new(Arc::new(key), None),
        )
        .await
        .expect("public-key authentication failed");
    assert!(matches!(auth, AuthResult::Success));

    let mut shell = handle
        .channel_open_session()
        .await
        .expect("open shell channel");
    shell
        .request_pty(true, "xterm", 80, 24, 0, 0, &[])
        .await
        .expect("request pty");
    shell
        .exec(true, "printf 'tabby-rs-ssh\\n'")
        .await
        .expect("exec shell command");
    let mut shell_output = Vec::new();
    let mut exit_status = None;
    while let Some(message) = timeout(Duration::from_secs(10), shell.wait())
        .await
        .expect("shell response timed out")
    {
        match message {
            ChannelMsg::Data { data } => shell_output.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => shell_output.extend_from_slice(&data),
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => exit_status = Some(status),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    assert_eq!(
        String::from_utf8_lossy(&shell_output).trim(),
        "tabby-rs-ssh"
    );
    assert_eq!(exit_status, Some(0));

    let channel = handle
        .channel_open_session()
        .await
        .expect("open SFTP channel");
    channel
        .request_subsystem(true, "sftp")
        .await
        .expect("request SFTP subsystem");
    let session = russh_sftp::client::SftpSession::new(channel.into_stream())
        .await
        .expect("start SFTP session");
    let mut sftp = SftpManager::new(session);

    sftp.mkdir("tabby-rs-fixture")
        .await
        .expect("create remote fixture directory");
    let listing = sftp
        .list("tabby-rs-fixture")
        .await
        .expect("list fixture directory");
    assert!(listing.is_empty());

    let cancelled = sftp
        .open_upload(
            "tabby-rs-fixture/cancelled.txt",
            None,
            SftpOverwritePolicy::Overwrite,
        )
        .await
        .expect("open cancellable upload");
    sftp.write(&cancelled.id, b"discarded")
        .await
        .expect("write cancellable upload");
    let cancelled = sftp.cancel(&cancelled.id).await.expect("cancel upload");
    assert_eq!(cancelled.state, "cancelled");

    let payload = b"real sftp payload\n";
    let upload = sftp
        .open_upload(
            "tabby-rs-fixture/data.txt",
            Some(payload.len() as u64),
            SftpOverwritePolicy::Overwrite,
        )
        .await
        .expect("open upload");
    sftp.write(&upload.id, payload).await.expect("write upload");
    let completed_upload = sftp.close(&upload.id).await.expect("close upload");
    assert_eq!(completed_upload.state, "completed");

    let entry = sftp
        .stat("tabby-rs-fixture/data.txt", true)
        .await
        .expect("stat uploaded file");
    assert_eq!(entry.size, payload.len() as u64);

    let download = sftp
        .open_download("tabby-rs-fixture/data.txt")
        .await
        .expect("open download");
    let (downloaded, _) = sftp.read(&download.id, 1024).await.expect("read download");
    assert_eq!(downloaded, payload);
    let (eof, _) = sftp
        .read(&download.id, 1024)
        .await
        .expect("read download EOF");
    assert!(eof.is_empty());
    sftp.close(&download.id).await.expect("close download");

    sftp.rename("tabby-rs-fixture/data.txt", "tabby-rs-fixture/renamed.txt")
        .await
        .expect("rename remote file");
    sftp.remove("tabby-rs-fixture", true)
        .await
        .expect("remove remote fixture directory");
    sftp.shutdown().await;
    handle
        .disconnect(Disconnect::ByApplication, "integration test complete", "")
        .await
        .expect("disconnect SSH session");
}
