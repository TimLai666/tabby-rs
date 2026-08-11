use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::{interval, timeout},
};

use crate::error::AppError;

use super::{
    codec::{TelnetCodec, TelnetEvent, TelnetNegotiator, IAC, NOP},
    model::{
        TelnetConnectRequest, TelnetEchoEvent, TelnetExitEvent, TelnetMessageEvent,
        TelnetOutputEvent, TelnetResizeRequest, TelnetSessionIdRequest, TelnetSessionInfo,
        TelnetWriteRequest,
    },
};

const MAX_WRITE_BYTES: usize = 1024 * 1024;
const MAX_CONNECT_TIMEOUT_MS: u64 = 120_000;
const MAX_KEEPALIVE_INTERVAL_MS: u64 = 3_600_000;
const READ_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct TelnetManager {
    sessions: Arc<Mutex<HashMap<String, TelnetSession>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct TelnetSession {
    control: mpsc::Sender<TelnetControl>,
}

enum TelnetControl {
    Write(Vec<u8>, oneshot::Sender<Result<(), AppError>>),
    Resize(TelnetResizeRequest, oneshot::Sender<Result<(), AppError>>),
    Close(oneshot::Sender<Result<(), AppError>>),
}

impl TelnetManager {
    pub async fn connect(
        &self,
        app: AppHandle,
        request: TelnetConnectRequest,
    ) -> Result<TelnetSessionInfo, AppError> {
        if request.profile_id.trim().is_empty() || request.host.trim().is_empty() {
            return Err(AppError::InvalidArgument(
                "Telnet profile and host are required".into(),
            ));
        }
        if request.host.len() > 255 || request.host.chars().any(char::is_control) {
            return Err(AppError::InvalidArgument("Telnet host is invalid".into()));
        }
        if request.port == 0 {
            return Err(AppError::InvalidArgument(
                "Telnet port must be greater than zero".into(),
            ));
        }
        let timeout_ms = request.connect_timeout_ms.clamp(1, MAX_CONNECT_TIMEOUT_MS);
        let host = request.host.clone();
        let port = request.port;
        let stream = timeout(
            Duration::from_millis(timeout_ms),
            TcpStream::connect((host.as_str(), port)),
        )
        .await
        .map_err(|_| AppError::Io(format!("Telnet connection to {host}:{port} timed out")))?
        .map_err(|error| {
            AppError::Io(format!(
                "Telnet connection to {host}:{port} failed: {error}"
            ))
        })?;
        stream.set_nodelay(true).map_err(AppError::from)?;

        let id = format!(
            "telnet-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let connection_id = request.connection_id.clone().unwrap_or_else(|| id.clone());
        let profile_id = request.profile_id.clone();
        let (control, controls) = mpsc::channel(32);
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), TelnetSession { control });

        let sessions = Arc::clone(&self.sessions);
        let task_id = id.clone();
        let task_profile_id = profile_id.clone();
        let task_connection_id = connection_id.clone();
        tokio::spawn(async move {
            run_connection(
                app,
                task_id.clone(),
                task_connection_id,
                task_profile_id,
                request,
                stream,
                controls,
            )
            .await;
            sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&task_id);
        });

        Ok(TelnetSessionInfo {
            id,
            profile_id,
            host,
            port,
        })
    }

    pub async fn write(&self, request: TelnetWriteRequest) -> Result<(), AppError> {
        if request.data.len() > MAX_WRITE_BYTES {
            return Err(AppError::InvalidArgument(
                "Telnet write is larger than 1 MiB".into(),
            ));
        }
        self.send(request.id, |sender| {
            TelnetControl::Write(request.data, sender)
        })
        .await
    }

    pub async fn resize(&self, request: TelnetResizeRequest) -> Result<(), AppError> {
        self.send(request.id.clone(), |sender| {
            TelnetControl::Resize(request, sender)
        })
        .await
    }

    pub async fn close(&self, request: TelnetSessionIdRequest) -> Result<(), AppError> {
        self.send(request.id, |sender| TelnetControl::Close(sender))
            .await
    }

    async fn send<F>(&self, id: String, make_control: F) -> Result<(), AppError>
    where
        F: FnOnce(oneshot::Sender<Result<(), AppError>>) -> TelnetControl,
    {
        let control = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("Telnet session {id} not found")))?
            .control;
        let (sender, receiver) = oneshot::channel();
        control
            .send(make_control(sender))
            .await
            .map_err(|_| AppError::Io("Telnet session is closed".into()))?;
        receiver
            .await
            .map_err(|_| AppError::Io("Telnet session stopped before replying".into()))?
    }
}

async fn run_connection(
    app: AppHandle,
    id: String,
    connection_id: String,
    profile_id: String,
    request: TelnetConnectRequest,
    stream: TcpStream,
    mut controls: mpsc::Receiver<TelnetControl>,
) {
    let _ = emit_message(
        &app,
        &id,
        &connection_id,
        &profile_id,
        format!("Connected to {}:{}", request.host, request.port),
    );
    let (mut reader, mut writer) = stream.into_split();
    let mut codec = TelnetCodec::new();
    let mut negotiator = TelnetNegotiator::new(request.terminal_type, request.local_echo);
    for bytes in negotiator.initial_requests() {
        if writer.write_all(&bytes).await.is_err() {
            emit_exit(
                &app,
                &id,
                &connection_id,
                &profile_id,
                "failed to send Telnet negotiation".into(),
            );
            return;
        }
    }
    emit_echo(
        &app,
        &id,
        &connection_id,
        &profile_id,
        negotiator.force_echo(),
    );

    let keepalive = request
        .keepalive
        .filter(|options| options.interval_ms > 0 && options.max_count > 0);
    let keepalive_interval = keepalive
        .as_ref()
        .map(|options| options.interval_ms.clamp(100, MAX_KEEPALIVE_INTERVAL_MS));
    let mut keepalive_tick = interval(Duration::from_millis(
        keepalive_interval.unwrap_or(3_600_000),
    ));
    keepalive_tick.tick().await;
    let mut idle_probes = 0_u32;
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    let reason = loop {
        tokio::select! {
            result = reader.read(&mut buffer) => {
                match result {
                    Ok(0) => break "remote closed the connection".into(),
                    Ok(length) => {
                        idle_probes = 0;
                        for event in codec.feed(&buffer[..length]) {
                            match event {
                                TelnetEvent::Data(data) if !data.is_empty() => {
                                    let _ = app.emit("telnet.output", TelnetOutputEvent {
                                        id: id.clone(), connection_id: connection_id.clone(), profile_id: profile_id.clone(), data,
                                    });
                                }
                                TelnetEvent::Command { .. } | TelnetEvent::Subnegotiation { .. } => {
                                    for response in negotiator.handle(&event) {
                                        if writer.write_all(&response).await.is_err() {
                                            break;
                                        }
                                    }
                                    emit_echo(&app, &id, &connection_id, &profile_id, negotiator.force_echo());
                                }
                                TelnetEvent::Malformed(message) => {
                                    let _ = emit_message(&app, &id, &connection_id, &profile_id, format!("Ignored malformed Telnet sequence: {message}"));
                                }
                                TelnetEvent::Data(_) => {}
                            }
                        }
                    }
                    Err(error) => break format!("read failed: {error}"),
                }
            }
            Some(control) = controls.recv() => {
                match control {
                    TelnetControl::Write(data, sender) => {
                        let encoded = encode_user_data(&data);
                        let result = writer.write_all(&encoded).await.map_err(AppError::from);
                        let should_stop = result.is_err();
                        let _ = sender.send(result);
                        if should_stop { break "write failed".into(); }
                    }
                    TelnetControl::Resize(request, sender) => {
                        let result = writer.write_all(&negotiator.resize(request.columns, request.rows)).await.map_err(AppError::from);
                        let should_stop = result.is_err();
                        let _ = sender.send(result);
                        if should_stop { break "resize negotiation failed".into(); }
                    }
                    TelnetControl::Close(sender) => {
                        let result = writer.shutdown().await.map_err(AppError::from);
                        let _ = sender.send(result);
                        break "closed by user".into();
                    }
                }
            }
            _ = keepalive_tick.tick(), if keepalive.is_some() => {
                idle_probes = idle_probes.saturating_add(1);
                if let Some(options) = &keepalive {
                    if idle_probes > options.max_count {
                        break "keepalive limit exceeded".into();
                    }
                }
                if writer.write_all(&[IAC, NOP]).await.is_err() {
                    break "keepalive write failed".into();
                }
            }
        }
    };
    emit_exit(&app, &id, &connection_id, &profile_id, reason);
}

fn encode_user_data(data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(data.len());
    for &byte in data {
        encoded.push(byte);
        if byte == IAC {
            encoded.push(IAC);
        }
    }
    encoded
}

fn emit_echo(app: &AppHandle, id: &str, connection_id: &str, profile_id: &str, force_echo: bool) {
    let _ = app.emit(
        "telnet.echo",
        TelnetEchoEvent {
            id: id.into(),
            connection_id: connection_id.into(),
            profile_id: profile_id.into(),
            force_echo,
        },
    );
}

fn emit_message(
    app: &AppHandle,
    id: &str,
    connection_id: &str,
    profile_id: &str,
    message: String,
) -> tauri::Result<()> {
    app.emit(
        "telnet.message",
        TelnetMessageEvent {
            id: id.into(),
            connection_id: connection_id.into(),
            profile_id: profile_id.into(),
            message,
        },
    )
}

fn emit_exit(app: &AppHandle, id: &str, connection_id: &str, profile_id: &str, reason: String) {
    let _ = app.emit(
        "telnet.exit",
        TelnetExitEvent {
            id: id.into(),
            connection_id: connection_id.into(),
            profile_id: profile_id.into(),
            reason,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::encode_user_data;
    use crate::telnet::codec::IAC;

    #[test]
    fn user_data_escapes_literal_iac_bytes() {
        assert_eq!(
            encode_user_data(&[1, IAC, 2, IAC]),
            vec![1, IAC, IAC, 2, IAC, IAC]
        );
    }
}
