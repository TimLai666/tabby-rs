use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use sha2::{Digest, Sha256};
use tabby_rs::update::{
    manifest::UpdateManifest,
    service::{verify_download, verify_signature},
};

const TEST_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\n\
RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
trusted comment: timestamp:1555779966\tfile:test\n\
QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

const TEST_PUBLIC_KEY: &str = "untrusted comment: minisign public key E7620F1842B4E81F\n\
RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

struct FakeUpdateServer {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeUpdateServer {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => serve_request(&mut stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FakeUpdateServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn serve_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > 16 * 1024 {
            return;
        }
    }

    let request_line = String::from_utf8_lossy(&request);
    let path = request_line
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let (status, content_type, body, location) = match path {
        "/stable" => (
            200,
            "application/json",
            manifest_json("stable", "1.0.231-tabbyrs.2", None),
            None,
        ),
        "/nightly" => (
            200,
            "application/json",
            manifest_json("nightly", "1.0.231-tabbyrs.3.nightly.20260812.1", None),
            None,
        ),
        "/rollback" => (
            200,
            "application/json",
            manifest_json("stable", "1.0.231-tabbyrs.1", None),
            None,
        ),
        "/wrong-arch" => (
            200,
            "application/json",
            manifest_json(
                "stable",
                "1.0.231-tabbyrs.2",
                Some(("arch", "unsupported-arch")),
            ),
            None,
        ),
        "/large-notes" => (
            200,
            "application/json",
            manifest_json(
                "stable",
                "1.0.231-tabbyrs.2",
                Some(("notes", &"x".repeat(65 * 1024))),
            ),
            None,
        ),
        "/malformed" => (
            200,
            "application/json",
            manifest_json(
                "stable",
                "1.0.231-tabbyrs.2",
                Some(("publishedAt", "not-a-date")),
            ),
            None,
        ),
        "/bad-signature" => (
            200,
            "application/json",
            manifest_json(
                "stable",
                "1.0.231-tabbyrs.2",
                Some(("signature", "not-a-minisign-signature")),
            ),
            None,
        ),
        "/redirect" => (302, "text/plain", "redirected".into(), Some("/stable")),
        "/artifact" => (200, "application/octet-stream", "test".into(), None),
        _ => (404, "text/plain", "not found".into(), None),
    };
    let status_text = match status {
        200 => "OK",
        302 => "Found",
        _ => "Not Found",
    };
    let mut response = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(location) = location {
        response.push_str(&format!("Location: {location}\r\n"));
    }
    response.push_str("\r\n");
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    if stream.write_all(&body).is_err() {
        return;
    }
    if stream.flush().is_err() {
        return;
    }
    let _ = stream.shutdown(Shutdown::Write);
}

fn manifest_json(channel: &str, version: &str, override_field: Option<(&str, &str)>) -> Vec<u8> {
    let artifact = b"test";
    let mut manifest = serde_json::json!({
        "schemaVersion": 1,
        "channel": channel,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "version": version,
        "url": "https://updates.example.test/tabby-rs.AppImage",
        "sha256": sha256_hex(artifact),
        "signature": TEST_SIGNATURE,
        "notes": "security fixes",
        "publishedAt": "2026-08-12T00:00:00Z",
        "size": artifact.len(),
    });
    if let Some((field, value)) = override_field {
        manifest[field] = serde_json::Value::String(value.into());
    }
    serde_json::to_vec(&manifest).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn get(server: &FakeUpdateServer, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(&server.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                server.address
            )
            .as_bytes(),
        )
        .unwrap();
    if let Err(error) = stream.shutdown(Shutdown::Write) {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotConnected,
            "request half-close failed unexpectedly: {error}"
        );
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("fixture response for {path} has no HTTP header: {response:?}"));
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, response[header_end + 4..].to_vec())
}

fn manifest(server: &FakeUpdateServer, path: &str) -> UpdateManifest {
    let (status, body) = get(server, path);
    assert_eq!(status, 200, "fixture route {path} must return a manifest");
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn fake_server_covers_update_manifest_artifact_and_failure_policy() {
    let server = FakeUpdateServer::start();

    let stable = manifest(&server, "/stable");
    let stable_version = stable
        .validate_for_channel("stable", "1.0.231-tabbyrs.1")
        .unwrap();
    assert!(stable_version.is_stable());
    let (_, artifact) = get(&server, "/artifact");
    verify_download(&artifact, &stable).unwrap();
    verify_signature(&artifact, &stable.signature, TEST_PUBLIC_KEY).unwrap();

    let nightly = manifest(&server, "/nightly");
    assert!(nightly
        .validate_for_channel("nightly", "1.0.231-tabbyrs.2")
        .is_ok());
    assert!(nightly
        .validate_for_channel("stable", "1.0.231-tabbyrs.2")
        .is_err());

    for path in ["/rollback", "/wrong-arch", "/large-notes", "/malformed"] {
        assert!(
            manifest(&server, path)
                .validate_for_channel("stable", "1.0.231-tabbyrs.2")
                .is_err(),
            "fixture route {path} must be rejected"
        );
    }

    let bad_signature = manifest(&server, "/bad-signature");
    assert!(verify_signature(&artifact, &bad_signature.signature, TEST_PUBLIC_KEY).is_err());

    let (status, _) = get(&server, "/redirect");
    assert_eq!(
        status, 302,
        "redirect fixture must remain visible to a no-redirect client"
    );
}
