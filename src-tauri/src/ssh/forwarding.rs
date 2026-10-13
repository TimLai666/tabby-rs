use std::{net::IpAddr, sync::Arc};

use tokio::io::{copy_bidirectional, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, watch},
};

use super::{
    model::{SshError, SshForwardingInfo, SshForwardingType},
    pending::CancelSignal,
    SshControl,
};

// Production local/dynamic listener setup; its I/O failures are preserved at the bridge.
pub(super) async fn bind_listener(
    host: &str,
    port: u16,
) -> Result<tokio::net::TcpListener, SshError> {
    tokio::net::TcpListener::bind((host, port))
        .await
        .map_err(|error| SshError::Forwarding(error.to_string()))
}

pub(crate) fn validate_endpoint(host: &str, port: u16, field: &str) -> Result<(), SshError> {
    if host.is_empty()
        || host.len() > 255
        || host.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\0'
        })
        || port == 0
    {
        return Err(SshError::InvalidRequest(format!("{field} is invalid")));
    }
    Ok(())
}

pub(crate) fn validate_bind_host(host: &str) -> Result<(), SshError> {
    validate_endpoint(host, 1, "forward bind host")
}

pub(crate) async fn socks5_connect<S>(stream: &mut S) -> Result<(String, u16), SshError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = stream.read_u8().await.map_err(|_| SshError::Closed)?;
    if version != 5 {
        return Err(SshError::InvalidRequest(
            "SOCKS5 greeting is invalid".into(),
        ));
    }
    let method_count = stream.read_u8().await.map_err(|_| SshError::Closed)? as usize;
    if method_count == 0 {
        return Err(SshError::InvalidRequest(
            "SOCKS5 greeting is invalid".into(),
        ));
    }
    let mut methods = vec![0; method_count];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|_| SshError::Closed)?;
    if !methods.contains(&0) {
        stream
            .write_all(&[5, 0xff])
            .await
            .map_err(|_| SshError::Closed)?;
        return Err(SshError::InvalidRequest(
            "SOCKS5 authentication is unsupported".into(),
        ));
    }
    stream
        .write_all(&[5, 0])
        .await
        .map_err(|_| SshError::Closed)?;

    if stream.read_u8().await.map_err(|_| SshError::Closed)? != 5 {
        return Err(SshError::InvalidRequest("SOCKS5 request is invalid".into()));
    }
    let command = stream.read_u8().await.map_err(|_| SshError::Closed)?;
    if !matches!(command, 1..=3) {
        return Err(SshError::InvalidRequest("SOCKS5 command is invalid".into()));
    }
    // The fixed upstream parser ignores the reserved byte.
    let _reserved = stream.read_u8().await.map_err(|_| SshError::Closed)?;
    let address_type = stream.read_u8().await.map_err(|_| SshError::Closed)?;
    let host = match address_type {
        1 => {
            let mut bytes = [0; 4];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            IpAddr::from(bytes).to_string()
        }
        3 => {
            let length = stream.read_u8().await.map_err(|_| SshError::Closed)? as usize;
            let mut bytes = vec![0; length];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        4 => {
            let mut bytes = [0; 16];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            bytes
                .chunks_exact(2)
                .map(|part| format!("{:04x}", u16::from_be_bytes([part[0], part[1]])))
                .collect::<Vec<_>>()
                .join(":")
        }
        _ => {
            return Err(SshError::InvalidRequest(
                "SOCKS5 address type is unsupported".into(),
            ));
        }
    };
    let port = stream.read_u16().await.map_err(|_| SshError::Closed)?;
    if command != 1 {
        send_socks5_failure(stream, 7).await?;
        return Err(SshError::InvalidRequest(
            "SOCKS5 request is unsupported".into(),
        ));
    }
    Ok((host, port))
}

pub(crate) async fn send_socks5_success<S>(stream: &mut S) -> Result<(), SshError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| SshError::Closed)
}

pub(crate) async fn send_socks5_failure<S>(stream: &mut S, code: u8) -> Result<(), SshError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[5, code])
        .await
        .map_err(|_| SshError::Closed)
}

#[cfg(test)]
mod tests {
    use super::socks5_connect;
    use tokio::io::duplex;

    #[tokio::test]
    async fn parses_domain_connect_request() {
        let (mut client, mut server) = duplex(1024);
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(
                &mut client,
                &[
                    5, 1, 0, 5, 1, 0, 3, 12, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b't',
                    b'e', b's', b't', 0, 22,
                ],
            )
            .await
            .unwrap();
            let mut response = [0; 2];
            tokio::io::AsyncReadExt::read_exact(&mut client, &mut response)
                .await
                .unwrap();
            assert_eq!(response, [5, 0]);
        });
        assert_eq!(
            socks5_connect(&mut server).await.unwrap(),
            ("example.test".into(), 22)
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_authentication_methods_other_than_no_auth() {
        let (mut client, mut server) = duplex(64);
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut client, &[5, 1, 2])
                .await
                .unwrap();
            let mut response = [0; 2];
            tokio::io::AsyncReadExt::read_exact(&mut client, &mut response)
                .await
                .unwrap();
            assert_eq!(response, [5, 0xff]);
        });
        assert!(socks5_connect(&mut server).await.is_err());
        writer.await.unwrap();
    }
}

// Send EOF before draining unread pipelined bytes, avoiding a TCP reset on rejection.
// Both callers run inside the forwarding cancellation signal.
async fn close_socks5_client(socket: &mut TcpStream) {
    if socket.shutdown().await.is_ok() {
        let _ = tokio::io::copy(socket, &mut tokio::io::sink()).await;
    }
}

pub(super) async fn run_local_forward(
    info: SshForwardingInfo,
    listener: TcpListener,
    control: mpsc::Sender<SshControl>,
    cancel: CancelSignal,
    on_message: impl Fn(String) + Send + Sync + 'static,
) {
    let on_message = Arc::new(on_message);
    let description = if info.kind == SshForwardingType::Dynamic {
        format!("(dynamic) {}:{}", info.bind_host, info.bind_port)
    } else {
        format!(
            "(local) {}:{} → (remote) {}:{}",
            info.bind_host, info.bind_port, info.target_address, info.target_port
        )
    };
    let description = super::terminal_diagnostic_text(&description);
    loop {
        let accepted = cancel
            .run(async { listener.accept().await.map_err(|_| SshError::Closed) })
            .await;
        let Ok((socket, peer)) = accepted else {
            break;
        };
        let control = control.clone();
        let cancel = cancel.clone();
        let kind = info.kind;
        let target_address = info.target_address.clone();
        let target_port = info.target_port;
        let on_message = on_message.clone();
        let description = description.clone();
        tokio::spawn(async move {
            let mut socket = socket;
            let _ = cancel
                .run(async {
                    let (target_address, target_port) = if kind == SshForwardingType::Dynamic {
                        match socks5_connect(&mut socket).await {
                            Ok(target) => target,
                            Err(_) => {
                                close_socks5_client(&mut socket).await;
                                return Ok(());
                            }
                        }
                    } else {
                        (target_address, target_port)
                    };
                    let (sender, receiver) = oneshot::channel();
                    control
                        .send(SshControl::OpenDirectTcpip {
                            host: target_address.clone(),
                            port: target_port,
                            originator: if kind == SshForwardingType::Dynamic {
                                ([127, 0, 0, 1], 0).into()
                            } else {
                                peer
                            },
                            cancel: cancel.clone(),
                            sender,
                        })
                        .await
                        .map_err(|_| SshError::Closed)?;
                    let mut ssh_stream = match receiver.await.map_err(|_| SshError::Closed)? {
                        Ok(stream) => stream,
                        Err(error) => {
                            if !cancel.is_cancelled() {
                                on_message(format!(
                                    "\x1b[41m\x1b[30m X \x1b[39m\x1b[49m Remote has rejected the forwarded connection to {}:{} via {}: {}",
                                    super::terminal_diagnostic_text(&target_address),
                                    target_port,
                                    description,
                                    super::terminal_diagnostic_text(&error.to_string()),
                                ));
                                if kind == SshForwardingType::Dynamic {
                                    let _ = send_socks5_failure(&mut socket, 2).await;
                                    close_socks5_client(&mut socket).await;
                                }
                            }
                            return Ok(());
                        }
                    };
                    if kind == SshForwardingType::Dynamic {
                        send_socks5_success(&mut socket).await?;
                    }
                    let _ = copy_bidirectional(&mut socket, &mut ssh_stream).await;
                    Ok(())
                })
                .await;
        });
    }
}

pub(super) fn dispatch_remote_forward(
    channel: russh::Channel<russh::client::Msg>,
    routes: &super::RemoteForwardRoutes,
    connection_id: &str,
    connected_address: &str,
    connected_port: u32,
    on_message: impl Fn(String) + Send + 'static,
) {
    let route = {
        let routes = routes.lock().unwrap_or_else(|error| error.into_inner());
        routes
            .get(&(
                connection_id.into(),
                connected_address.into(),
                connected_port,
            ))
            .cloned()
            .or_else(|| {
                routes
                    .get(&(connection_id.into(), connected_address.into(), 0))
                    .filter(|route| {
                        route
                            .ready
                            .as_ref()
                            .is_some_and(|ready| ready.borrow().is_none())
                    })
                    .cloned()
            })
    };
    let stream = channel.into_stream();
    let connected_address = connected_address.to_owned();
    tokio::spawn(async move {
        run_remote_forward(
            stream,
            route,
            &connected_address,
            connected_port,
            on_message,
        )
        .await
    });
}

pub(super) async fn run_remote_forward(
    mut ssh_stream: russh::ChannelStream<russh::client::Msg>,
    route: Option<Arc<super::RemoteForwardRoute>>,
    connected_address: &str,
    connected_port: u32,
    on_message: impl Fn(String) + Send,
) -> Result<(), russh::Error> {
    let rejection_message = || {
        format!(
            "\x1b[41m\x1b[30m X \x1b[39m\x1b[49m Rejected incoming forwarded connection for unrecognized port {}:{connected_port}",
            super::terminal_diagnostic_text(connected_address),
        )
    };
    let Some(route) = route else {
        on_message(rejection_message());
        return Ok::<(), russh::Error>(());
    };
    if let Some(ready) = route.ready.clone() {
        match route.cancel.run(wait_for_remote_forward_ready(ready)).await {
            Ok(actual) if actual == connected_port => {}
            Ok(_) => {
                on_message(rejection_message());
                return Ok::<(), russh::Error>(());
            }
            Err(_) => return Ok::<(), russh::Error>(()),
        }
    }
    let target = route
        .cancel
        .run(async {
            Ok(TcpStream::connect((route.target_address.as_str(), route.target_port)).await)
        })
        .await;
    let mut target = match target {
        Ok(Ok(target)) => target,
        Ok(Err(error)) => {
            on_message(format!(
                "\x1b[41m\x1b[30m X \x1b[39m\x1b[49m Could not forward the remote connection to {}:{}: {}",
                super::terminal_diagnostic_text(&route.target_address),
                route.target_port,
                super::terminal_diagnostic_text(&error.to_string()),
            ));
            let _ = ssh_stream.shutdown().await;
            return Ok::<(), russh::Error>(());
        }
        Err(_) => return Ok::<(), russh::Error>(()),
    };
    let _ = route
        .cancel
        .run(async {
            let _ = copy_bidirectional(&mut target, &mut ssh_stream).await;
            Ok(())
        })
        .await;
    Ok::<(), russh::Error>(())
}

async fn wait_for_remote_forward_ready(
    mut ready: watch::Receiver<Option<u32>>,
) -> Result<u32, SshError> {
    loop {
        if let Some(port) = *ready.borrow_and_update() {
            return Ok(port);
        }
        ready.changed().await.map_err(|_| SshError::Closed)?;
    }
}
