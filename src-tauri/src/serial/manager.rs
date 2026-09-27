use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serialport::{DataBits, SerialPort, StopBits};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use crate::error::AppError;

use super::{
    codec::MAX_SERIAL_CHUNK_BYTES,
    enumerate::{list_ports, path_for_stable_id, stable_id},
    model::{
        SerialBaudRateRequest, SerialConnectionStateEvent, SerialOpenRequest, SerialOutputEvent,
        SerialPortInfo, SerialSessionIdRequest, SerialSessionInfo, SerialSignal, SerialSignalRequest,
        SerialSignalState, SerialWriteRequest,
    },
};

const MAX_WRITE_BYTES: usize = 1024 * 1024;
const MAX_READ_TIMEOUT_MS: u64 = 1_000;
const MAX_RECONNECT_ATTEMPTS: u32 = 20;
const MAX_RECONNECT_DELAY_MS: u64 = 60_000;

#[derive(Clone, Default)]
pub struct SerialManager {
    sessions: Arc<Mutex<HashMap<String, SerialSession>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Clone)]
struct SerialSession {
    control: Sender<SerialControl>,
    writer: SerialWriter,
}

#[derive(Clone)]
struct SerialWriter {
    port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
    closed: Arc<AtomicBool>,
    disconnected: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
}

impl SerialWriter {
    fn new(port: Box<dyn SerialPort>) -> Self {
        Self {
            port: Arc::new(Mutex::new(Some(port))),
            closed: Arc::new(AtomicBool::new(false)),
            disconnected: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn write(&self, data: &[u8], generation: u64) -> Result<(), AppError> {
        let mut port = self.port.lock().unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return Err(AppError::Io("Serial session is closed".into()));
        }
        if self.generation.load(Ordering::Acquire) != generation {
            return Err(AppError::Io("Serial port changed before writing".into()));
        }
        if self.disconnected.load(Ordering::Acquire) {
            return Err(AppError::Io("Serial port is disconnected".into()));
        }
        let port = port.as_mut()
            .ok_or_else(|| AppError::Io("Serial port is disconnected".into()))?;
        write_serial_data(port.as_mut(), data, &self.closed, &self.disconnected).map_err(AppError::from)
    }

    fn replace(&self, port: Option<Box<dyn SerialPort>>) {
        if port.is_none() {
            self.disconnected.store(true, Ordering::Release);
        }
        let mut current = self.port.lock().unwrap_or_else(|error| error.into_inner());
        self.generation.fetch_add(1, Ordering::Release);
        *current = if self.closed.load(Ordering::Acquire) { None } else { port };
        self.disconnected.store(current.is_none(), Ordering::Release);
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

enum SerialControl {
    SetBaudRate(u32, oneshot::Sender<Result<(), AppError>>),
    SetSignals(SerialSignalRequest, oneshot::Sender<Result<(), AppError>>),
    GetSignals(oneshot::Sender<Result<SerialSignalState, AppError>>),
    Close(oneshot::Sender<Result<(), AppError>>),
}

impl SerialManager {
    pub fn start_port_watcher(app: AppHandle) {
        thread::spawn(move || {
            let mut previous = list_ports().unwrap_or_default();
            loop {
                thread::sleep(Duration::from_secs(2));
                let Ok(current) = list_ports() else {
                    continue;
                };
                if current != previous {
                    let _ = app.emit("serial:portsChanged", current.clone());
                    previous = current;
                }
            }
        });
    }

    pub fn open(
        &self,
        app: AppHandle,
        request: SerialOpenRequest,
    ) -> Result<SerialSessionInfo, AppError> {
        validate_open_request(&request)?;
        let port_info = find_port(&request.port)?;
        let stable = port_info
            .as_ref()
            .map(stable_id)
            .unwrap_or_else(|| format!("path:{}", request.port));
        let serial = open_port(&request, &request.port)?;
        let writer = SerialWriter::new(serial.try_clone().map_err(serial_error)?);
        let id = format!(
            "serial-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (control, controls) = mpsc::channel();
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), SerialSession { control, writer: writer.clone() });

        let sessions = Arc::clone(&self.sessions);
        let task_id = id.clone();
        let response = SerialSessionInfo {
            id: id.clone(),
            profile_id: request.profile_id.clone(),
            port: request.port.clone(),
            stable_id: stable.clone(),
        };
        thread::spawn(move || {
            run_session(app, task_id.clone(), request, stable, serial, controls, &writer);
            writer.close();
            writer.replace(None);
            sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&task_id);
        });
        Ok(response)
    }

    pub async fn write(&self, request: SerialWriteRequest) -> Result<(), AppError> {
        if request.data.len() > MAX_WRITE_BYTES {
            return Err(AppError::InvalidArgument(
                "Serial write is larger than 1 MiB".into(),
            ));
        }
        let writer = self.session(&request.id)?.writer;
        let generation = writer.generation.load(Ordering::Acquire);
        tokio::task::spawn_blocking(move || writer.write(&request.data, generation))
            .await
            .map_err(|error| AppError::Io(error.to_string()))?
    }

    pub async fn set_baud_rate(&self, request: SerialBaudRateRequest) -> Result<(), AppError> {
        if request.baud_rate == 0 {
            return Err(AppError::InvalidArgument(
                "Baud rate must be greater than zero".into(),
            ));
        }
        self.send(request.id, |sender| {
            SerialControl::SetBaudRate(request.baud_rate, sender)
        })
        .await
    }

    pub async fn set_signals(&self, request: SerialSignalRequest) -> Result<(), AppError> {
        self.send(request.id.clone(), |sender| {
            SerialControl::SetSignals(request, sender)
        })
        .await
    }

    pub async fn get_signals(
        &self,
        request: SerialSessionIdRequest,
    ) -> Result<SerialSignalState, AppError> {
        let control = self.control(&request.id)?;
        let (sender, receiver) = oneshot::channel();
        control
            .send(SerialControl::GetSignals(sender))
            .map_err(|_| AppError::Io("Serial session is closed".into()))?;
        receiver
            .await
            .map_err(|_| AppError::Io("Serial session stopped before replying".into()))?
    }

    pub async fn close(&self, request: SerialSessionIdRequest) -> Result<(), AppError> {
        self.session(&request.id)?.writer.close();
        self.send(request.id, |sender| SerialControl::Close(sender))
            .await
    }

    async fn send<F>(&self, id: String, make_control: F) -> Result<(), AppError>
    where
        F: FnOnce(oneshot::Sender<Result<(), AppError>>) -> SerialControl,
    {
        let control = self.control(&id)?;
        let (sender, receiver) = oneshot::channel();
        control
            .send(make_control(sender))
            .map_err(|_| AppError::Io("Serial session is closed".into()))?;
        receiver
            .await
            .map_err(|_| AppError::Io("Serial session stopped before replying".into()))?
    }

    fn control(&self, id: &str) -> Result<Sender<SerialControl>, AppError> {
        Ok(self.session(id)?.control)
    }

    fn session(&self, id: &str) -> Result<SerialSession, AppError> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("Serial session {id} not found")))
    }
}

pub fn list_serial_ports() -> Result<Vec<SerialPortInfo>, AppError> {
    list_ports()
}

fn validate_open_request(request: &SerialOpenRequest) -> Result<(), AppError> {
    if request.profile_id.trim().is_empty()
        || request.connection_id.trim().is_empty()
        || request.port.trim().is_empty()
    {
        return Err(AppError::InvalidArgument(
            "Serial profile, connection, and port are required".into(),
        ));
    }
    if request.port.len() > 1024 || request.port.chars().any(char::is_control) {
        return Err(AppError::InvalidArgument(
            "Serial port path is invalid".into(),
        ));
    }
    if request.baud_rate == 0 {
        return Err(AppError::InvalidArgument(
            "Serial baud rate must be greater than zero".into(),
        ));
    }
    if !matches!(request.data_bits, 5..=8) || !matches!(request.stop_bits, 1.0 | 1.5 | 2.0) {
        return Err(AppError::InvalidArgument(
            "Serial data bits or stop bits are invalid".into(),
        ));
    }
    if matches!(
        request.parity,
        super::model::SerialParity::Mark | super::model::SerialParity::Space
    ) {
        return Err(AppError::Unsupported(
            "Serial mark and space parity are not supported by the native backend".into(),
        ));
    }
    Ok(())
}

fn find_port(path: &str) -> Result<Option<SerialPortInfo>, AppError> {
    Ok(list_ports()
        .unwrap_or_default()
        .into_iter()
        .find(|port| port.path == path))
}

fn open_port(request: &SerialOpenRequest, path: &str) -> Result<Box<dyn SerialPort>, AppError> {
    let data_bits = match request.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        8 => DataBits::Eight,
        _ => unreachable!("validated data bits"),
    };
    let stop_bits = if (request.stop_bits - 1.5).abs() < f32::EPSILON {
        return Err(AppError::Unsupported(
            "Serial 1.5 stop bits are not supported by the native backend".into(),
        ));
    } else if (request.stop_bits - 2.0).abs() < f32::EPSILON {
        StopBits::Two
    } else {
        StopBits::One
    };
    let port = serialport::new(path, request.baud_rate)
        .data_bits(data_bits)
        .stop_bits(stop_bits)
        .parity(request.parity.as_serialport())
        .flow_control(request.flow_control.as_serialport())
        .timeout(Duration::from_millis(
            request.read_timeout_ms.clamp(10, MAX_READ_TIMEOUT_MS),
        ))
        .open_native()
        .map_err(|error| AppError::Io(format!("failed to open serial port {path}: {error}")))?;
    prepare_serial_port(port)
}

#[cfg(unix)]
fn prepare_serial_port(port: serialport::TTYPort) -> Result<Box<dyn SerialPort>, AppError> {
    use std::os::fd::AsRawFd;
    // Readiness polling alone cannot bound a large write to a blocking tty.
    // Keep the descriptor nonblocking, as upstream's Unix binding does. The
    // SerialPort implementation still polls using the configured timeout.
    let fd = port.as_raw_fd();
    // SAFETY: fd belongs to the live port; these fcntl operations take no pointers.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Box::new(port))
}

#[cfg(windows)]
fn prepare_serial_port(port: serialport::COMPort) -> Result<Box<dyn SerialPort>, AppError> {
    Ok(Box::new(port))
}

fn run_session(
    app: AppHandle,
    id: String,
    mut request: SerialOpenRequest,
    stable: String,
    mut port: Box<dyn SerialPort>,
    controls: Receiver<SerialControl>,
    writer: &SerialWriter,
) {
    let mut current_path = request.port.clone();
    let mut buffer = vec![0_u8; MAX_SERIAL_CHUNK_BYTES];
    let mut reconnect_attempts = 0_u32;
    let mut waiting_emitted = false;
    emit_state(&app, &id, &request, "connected", Some(&current_path), None);

    loop {
        match controls.try_recv() {
            Ok(control) => {
                if handle_control(&mut port, control, writer, &mut request.baud_rate) {
                    emit_state(&app, &id, &request, "closed", Some(&current_path), None);
                    return;
                }
            }
            Err(TryRecvError::Disconnected) => {
                emit_state(&app, &id, &request, "closed", Some(&current_path), None);
                return;
            }
            Err(TryRecvError::Empty) => {}
        }

        match port.read(&mut buffer) {
            Ok(length) if length > 0 => {
                let _ = app.emit(
                    "serial:output",
                    SerialOutputEvent {
                        id: id.clone(),
                        connection_id: request.connection_id.clone(),
                        profile_id: request.profile_id.clone(),
                        data: buffer[..length].to_vec(),
                    },
                );
                reconnect_attempts = 0;
            }
            Ok(_) => {}
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {}
            Err(error) => {
                let message = error.to_string();
                emit_state(
                    &app,
                    &id,
                    &request,
                    "disconnected",
                    Some(&current_path),
                    Some(&message),
                );
                writer.replace(None);
                drop(port);
                if !request.reconnect.enabled {
                    return;
                }
                loop {
                    match controls.recv_timeout(Duration::from_millis(reconnect_delay(
                        reconnect_attempts,
                        request.reconnect.max_delay_ms,
                    ))) {
                        Ok(control) => {
                            if handle_control_without_port(
                                control,
                                &mut request.baud_rate,
                                &mut reconnect_attempts,
                            ) {
                                emit_state(&app, &id, &request, "closed", None, None);
                                return;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            emit_state(&app, &id, &request, "closed", None, None);
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let next_path = path_for_stable_id(&stable);
                            if next_path.is_none() {
                                if !waiting_emitted {
                                    emit_state(&app, &id, &request, "waiting", None, None);
                                    waiting_emitted = true;
                                }
                                continue;
                            }
                            reconnect_attempts = reconnect_attempts.saturating_add(1);
                            if reconnect_attempts
                                > request
                                    .reconnect
                                    .max_attempts
                                    .max(1)
                                    .min(MAX_RECONNECT_ATTEMPTS)
                            {
                                if !waiting_emitted {
                                    emit_state(&app, &id, &request, "waiting", None, None);
                                    waiting_emitted = true;
                                }
                                continue;
                            }
                            let path = next_path.unwrap_or_else(|| current_path.clone());
                            emit_state(&app, &id, &request, "reconnecting", Some(&path), None);
                            match open_port(&request, &path).and_then(|port| {
                                port.try_clone().map(|writer| (port, writer)).map_err(serial_error)
                            }) {
                                Ok((new_port, new_writer)) => {
                                    current_path = path;
                                    port = new_port;
                                    writer.replace(Some(new_writer));
                                    waiting_emitted = false;
                                    reconnect_attempts = 0;
                                    emit_state(
                                        &app,
                                        &id,
                                        &request,
                                        "connected",
                                        Some(&current_path),
                                        None,
                                    );
                                    break;
                                }
                                Err(error) => {
                                    emit_state(
                                        &app,
                                        &id,
                                        &request,
                                        "reconnecting",
                                        Some(&path),
                                        Some(&error.to_string()),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn handle_control(
    port: &mut Box<dyn SerialPort>,
    control: SerialControl,
    writer: &SerialWriter,
    baud_rate: &mut u32,
) -> bool {
    match control {
        SerialControl::SetBaudRate(rate, sender) => {
            let result = if writer.closed.load(Ordering::Acquire) {
                Err(AppError::Io("Serial session is closed".into()))
            } else {
                port.set_baud_rate(rate).map_err(serial_error)
            };
            if result.is_ok() {
                *baud_rate = rate;
            }
            let _ = sender.send(result);
            false
        }
        SerialControl::SetSignals(request, sender) => {
            let result = set_signal(port.as_mut(), request.signal, request.value);
            let _ = sender.send(result);
            false
        }
        SerialControl::GetSignals(sender) => {
            let result = get_signals(port.as_mut());
            let _ = sender.send(result);
            false
        }
        SerialControl::Close(sender) => {
            writer.close();
            writer.replace(None);
            let _ = sender.send(Ok(()));
            true
        }
    }
}

fn handle_control_without_port(
    control: SerialControl,
    baud_rate: &mut u32,
    reconnect_attempts: &mut u32,
) -> bool {
    match control {
        SerialControl::SetBaudRate(rate, sender) => {
            *baud_rate = rate;
            *reconnect_attempts = 0;
            let _ = sender.send(Ok(()));
            false
        }
        SerialControl::Close(sender) => {
            let _ = sender.send(Ok(()));
            true
        }
        SerialControl::SetSignals(_, sender) => {
            let _ = sender.send(Err(AppError::Io("Serial port is disconnected".into())));
            false
        }
        SerialControl::GetSignals(sender) => {
            let _ = sender.send(Err(AppError::Io("Serial port is disconnected".into())));
            false
        }
    }
}

fn set_signal(
    port: &mut dyn SerialPort,
    signal: SerialSignal,
    value: bool,
) -> Result<(), AppError> {
    match signal {
        SerialSignal::RequestToSend => port.write_request_to_send(value),
        SerialSignal::DataTerminalReady => port.write_data_terminal_ready(value),
    }
    .map_err(serial_error)
}

fn get_signals(port: &mut dyn SerialPort) -> Result<SerialSignalState, AppError> {
    Ok(SerialSignalState {
        clear_to_send: port.read_clear_to_send().map_err(serial_error)?,
        data_set_ready: port.read_data_set_ready().map_err(serial_error)?,
    })
}

fn reconnect_delay(attempt: u32, max_delay_ms: u64) -> u64 {
    let max_delay_ms = max_delay_ms.clamp(100, MAX_RECONNECT_DELAY_MS);
    1_000_u64
        .saturating_mul(2_u64.saturating_pow(attempt.min(6)))
        .min(max_delay_ms)
}

fn emit_state(
    app: &AppHandle,
    id: &str,
    request: &SerialOpenRequest,
    state: &str,
    path: Option<&str>,
    error: Option<&str>,
) {
    let _ = app.emit(
        "serial:connectionState",
        SerialConnectionStateEvent {
            id: id.into(),
            connection_id: request.connection_id.clone(),
            profile_id: request.profile_id.clone(),
            state: state.into(),
            path: path.map(str::to_owned),
            error: error.map(str::to_owned),
        },
    );
}

fn serial_error(error: serialport::Error) -> AppError {
    AppError::Io(error.to_string())
}

fn write_serial_data<W: Write + ?Sized>(port: &mut W, data: &[u8], closed: &AtomicBool, disconnected: &AtomicBool) -> std::io::Result<()> {
    let mut remaining = data;
    while !remaining.is_empty() {
        if closed.load(Ordering::Acquire) {
            return Err(std::io::Error::new(ErrorKind::BrokenPipe, "Serial session is closed"));
        }
        if disconnected.load(Ordering::Acquire) {
            return Err(std::io::Error::new(ErrorKind::BrokenPipe, "Serial port is disconnected"));
        }
        match port.write(remaining) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(length) => remaining = &remaining[length..],
            Err(error) if matches!(error.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
