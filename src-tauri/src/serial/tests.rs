#[test]
fn write_failures_and_cancellation_preserve_the_accepted_prefix() {
    use super::{write_serial_data, AtomicBool, Ordering};
    use std::{
        collections::VecDeque,
        io::{self, ErrorKind, Write},
    };
    struct Writer<'a> {
        answers: VecDeque<Result<usize, ErrorKind>>,
        accepted: Vec<u8>,
        attempts: usize,
        cancel: bool,
        closed: &'a AtomicBool,
    }
    impl Write for Writer<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.attempts += 1;
            let count = self
                .answers
                .pop_front()
                .unwrap_or(Ok(bytes.len()))
                .map_err(io::Error::from)?
                .min(bytes.len());
            self.accepted.extend_from_slice(&bytes[..count]);
            if self.cancel {
                self.closed.store(true, Ordering::Release);
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            panic!("serial writes must not flush")
        }
    }
    for (answers, cancel, accepted, failure, attempts) in [
        (vec![Ok(2), Ok(1)], false, "abcd", None, 3),
        (vec![Err(ErrorKind::Interrupted)], false, "abcd", None, 2),
        (vec![Err(ErrorKind::WouldBlock)], false, "abcd", None, 2),
        (vec![Ok(0)], false, "", Some(ErrorKind::WriteZero), 1),
        (
            vec![Ok(1), Err(ErrorKind::BrokenPipe)],
            false,
            "a",
            Some(ErrorKind::BrokenPipe),
            2,
        ),
        (vec![Ok(1)], true, "a", Some(ErrorKind::BrokenPipe), 1),
    ] {
        let closed = AtomicBool::new(false);
        let mut writer = Writer {
            answers: answers.into(),
            accepted: vec![],
            attempts: 0,
            cancel,
            closed: &closed,
        };
        let disconnected = AtomicBool::new(false);
        let result = write_serial_data(&mut writer, b"abcd", &closed, &disconnected);
        assert_eq!(result.err().map(|error| error.kind()), failure);
        assert_eq!(writer.accepted, accepted.as_bytes());
        assert_eq!(writer.attempts, attempts);
        write_serial_data(&mut writer, b"", &closed, &disconnected).unwrap();
        assert_eq!(writer.attempts, attempts, "empty input does not write");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn baud_rate_control_preserves_the_live_port_and_reconnect_settings() {
    use super::{
        handle_control, SerialBaudRateRequest, SerialManager, SerialSession, SerialWriter,
    };
    use serialport::SerialPort;
    use std::{
        io::{Read, Write},
        sync::mpsc,
        time::Duration,
    };
    let (mut peer, port) = serialport::TTYPort::pair().unwrap();
    peer.set_timeout(Duration::from_secs(1)).unwrap();
    let mut port = super::prepare_serial_port(port).unwrap();
    let writer = SerialWriter::new(port.try_clone().unwrap());
    let manager = SerialManager::default();
    let (control, controls) = mpsc::channel();
    manager.sessions.lock().unwrap().insert(
        "fixture".into(),
        SerialSession {
            control,
            writer: writer.clone(),
        },
    );
    // A rate change must not wait for the write handle (which may be stalled).
    let write_guard = writer.port.lock().unwrap();
    let worker_writer = writer.clone();
    let worker = std::thread::spawn(move || {
        let mut baud_rate = 115200;
        let name = port.name();
        assert!(!handle_control(
            &mut port,
            controls.recv().unwrap(),
            &worker_writer,
            &mut baud_rate
        ));
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(
                baud_rate, 9600,
                "reconnect request retains the successful rate"
            );
            assert_eq!(port.baud_rate().unwrap(), 9600);
        }
        // macOS pseudo-terminals reject the driver's IOSSIOSPEED ioctl. This
        // exercises actual driver failure, not physical-device success.
        #[cfg(target_os = "macos")]
        assert_eq!(
            baud_rate, 115200,
            "driver failure must not change reconnect settings"
        );
        assert_eq!(port.name(), name);
        port.write_all(b"same port").unwrap();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        manager.set_baud_rate(SerialBaudRateRequest {
            id: "fixture".into(),
            baud_rate: 9600,
        }),
    )
    .await
    .unwrap();
    drop(write_guard);
    #[cfg(target_os = "macos")]
    assert!(result.unwrap_err().to_string().contains("Not a typewriter"));
    #[cfg(not(target_os = "macos"))]
    result.unwrap();
    let mut data = [0; 9];
    peer.read_exact(&mut data).unwrap();
    assert_eq!(&data, b"same port");
    worker.join().unwrap();
}

#[tokio::test]
async fn baud_rate_rejects_invalid_requests_and_updates_the_next_reconnect() {
    use super::{handle_control_without_port, SerialBaudRateRequest, SerialControl, SerialManager};
    use tokio::sync::oneshot;
    let manager = SerialManager::default();
    for baud_rate in [0, 9600] {
        let error = manager
            .set_baud_rate(SerialBaudRateRequest {
                id: "missing".into(),
                baud_rate,
            })
            .await
            .unwrap_err();
        if baud_rate == 0 {
            assert!(matches!(error, crate::error::AppError::InvalidArgument(_)));
        } else {
            assert!(matches!(error, crate::error::AppError::NotFound(_)));
        }
    }
    let (sender, receiver) = oneshot::channel();
    let mut baud_rate = 115200;
    let mut reconnect_attempts = 20;
    assert!(!handle_control_without_port(
        SerialControl::SetBaudRate(9600, sender),
        &mut baud_rate,
        &mut reconnect_attempts
    ));
    receiver.await.unwrap().unwrap();
    assert_eq!(baud_rate, 9600, "the next open must use the selected rate");
    assert_eq!(
        reconnect_attempts, 0,
        "changing settings starts a fresh bounded retry sequence"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn writes_do_not_wait_for_read_controls() {
    use super::{SerialManager, SerialSession, SerialWriteRequest, SerialWriter};
    use serialport::SerialPort;
    use std::{
        io::{Read, Write},
        sync::mpsc,
        time::Duration,
    };
    let (mut peer, mut reader) = serialport::TTYPort::pair().unwrap();
    reader.set_timeout(Duration::from_secs(2)).unwrap();
    peer.set_timeout(Duration::from_secs(2)).unwrap();
    let manager = SerialManager::default();
    let (control, _controls) = mpsc::channel();
    manager.sessions.lock().unwrap().insert(
        "fixture".into(),
        SerialSession {
            control,
            writer: SerialWriter::new(
                super::prepare_serial_port(reader.try_clone_native().unwrap()).unwrap(),
            ),
        },
    );
    let (entered, ready) = mpsc::channel();
    let reading = std::thread::spawn(move || {
        entered.send(()).unwrap();
        let mut byte = [0];
        reader.read_exact(&mut byte).unwrap();
        byte
    });
    ready.recv().unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        manager.write(SerialWriteRequest {
            id: "fixture".into(),
            data: vec![0, 255, 65],
        }),
    )
    .await;
    peer.write_all(b"R").unwrap();
    assert_eq!(reading.join().unwrap(), [b'R']);
    result
        .expect("write must complete while the reader is waiting")
        .unwrap();
    let mut received = [0; 3];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(received, [0, 255, 65]);
}

#[cfg(unix)]
#[test]
fn disconnected_writer_reconnects_and_close_cannot_be_undone() {
    use super::SerialWriter;
    use serialport::SerialPort;
    use std::{io::Read, time::Duration};
    let (mut first_peer, first) = serialport::TTYPort::pair().unwrap();
    first_peer.set_timeout(Duration::from_secs(1)).unwrap();
    let writer = SerialWriter::new(super::prepare_serial_port(first).unwrap());
    writer.write(b"A", 0).unwrap();
    let mut received = [0];
    first_peer.read_exact(&mut received).unwrap();
    assert_eq!(received, [b'A']);
    writer.replace(None);
    assert!(writer.write(b"B", 1).is_err());

    let (mut second_peer, second) = serialport::TTYPort::pair().unwrap();
    second_peer.set_timeout(Duration::from_secs(1)).unwrap();
    writer.replace(Some(super::prepare_serial_port(second).unwrap()));
    assert!(writer.write(b"STALE", 0).is_err());
    writer.write(b"C", 2).unwrap();
    second_peer.read_exact(&mut received).unwrap();
    assert_eq!(received, [b'C']);
    let pending = writer.clone();
    writer.close();
    assert!(pending.write(b"D", 2).is_err());
    let (_, replacement) = serialport::TTYPort::pair().unwrap();
    writer.replace(Some(super::prepare_serial_port(replacement).unwrap()));
    assert!(writer.port.lock().unwrap().is_none());
    assert!(writer.write(b"E", 3).is_err());
}

#[tokio::test]
async fn missing_sessions_and_oversized_writes_are_rejected() {
    use super::{SerialManager, SerialWriteRequest, MAX_WRITE_BYTES};
    let manager = SerialManager::default();
    for data in [vec![1], vec![0; MAX_WRITE_BYTES + 1]] {
        assert!(manager
            .write(SerialWriteRequest {
                id: "missing".into(),
                data
            })
            .await
            .is_err());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn output_is_readable_while_a_large_write_is_stalled() {
    use super::{SerialManager, SerialSession, SerialWriteRequest, SerialWriter, MAX_WRITE_BYTES};
    use serialport::SerialPort;
    use std::{
        io::{Read, Write},
        sync::mpsc,
        time::Duration,
    };
    let (mut peer, mut reader) = serialport::TTYPort::pair().unwrap();
    peer.set_timeout(Duration::from_secs(3)).unwrap();
    reader.set_timeout(Duration::from_millis(50)).unwrap();
    let writer =
        SerialWriter::new(super::prepare_serial_port(reader.try_clone_native().unwrap()).unwrap());
    let manager = SerialManager::default();
    let (control, _controls) = mpsc::channel();
    manager.sessions.lock().unwrap().insert(
        "fixture".into(),
        SerialSession {
            control,
            writer: writer.clone(),
        },
    );
    let writing = tokio::spawn(async move {
        manager
            .write(SerialWriteRequest {
                id: "fixture".into(),
                data: vec![0xa5; MAX_WRITE_BYTES],
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while writer.port.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("writer entered the blocking write");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !writing.is_finished(),
        "flow control may stall longer than the read timeout"
    );
    peer.write_all(b"OUTPUT").unwrap();
    let mut output = [0; 6];
    reader.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"OUTPUT");
    assert!(
        !writing.is_finished(),
        "the peer has not drained the write yet"
    );
    let draining = std::thread::spawn(move || {
        let mut bytes = vec![0; MAX_WRITE_BYTES];
        peer.read_exact(&mut bytes).unwrap();
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
    });
    tokio::time::timeout(Duration::from_secs(4), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    draining.join().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn close_waits_until_the_writer_stops() {
    use super::{
        handle_control, SerialManager, SerialSession, SerialSessionIdRequest, SerialWriteRequest,
        SerialWriter, MAX_WRITE_BYTES,
    };
    use serialport::SerialPort;
    use std::{io::Read, sync::mpsc, time::Duration};
    let (mut peer, mut port) = serialport::TTYPort::pair().unwrap();
    peer.set_timeout(Duration::from_millis(200)).unwrap();
    port.set_timeout(Duration::from_millis(200)).unwrap();
    let writer =
        SerialWriter::new(super::prepare_serial_port(port.try_clone_native().unwrap()).unwrap());
    let manager = SerialManager::default();
    let (control, controls) = mpsc::channel();
    manager.sessions.lock().unwrap().insert(
        "fixture".into(),
        SerialSession {
            control,
            writer: writer.clone(),
        },
    );
    let close_writer = writer.clone();
    let closing = std::thread::spawn(move || {
        let mut port: Box<dyn SerialPort> = Box::new(port);
        assert!(handle_control(
            &mut port,
            controls.recv().unwrap(),
            &close_writer,
            &mut 115200
        ));
    });
    let send_manager = manager.clone();
    let writing = tokio::spawn(async move {
        send_manager
            .write(SerialWriteRequest {
                id: "fixture".into(),
                data: vec![0xa5; MAX_WRITE_BYTES],
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while writer.port.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(2),
        manager.close(SerialSessionIdRequest {
            id: "fixture".into(),
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let released_before_ack = writer.port.try_lock().is_ok();
    let draining = std::thread::spawn(move || {
        let mut count = 0;
        let mut buffer = [0; 8192];
        while let Ok(n) = peer.read(&mut buffer) {
            if n == 0 {
                break;
            }
            count += n;
        }
        count
    });
    let result = writing.await.unwrap();
    let count = draining.join().unwrap();
    closing.join().unwrap();
    assert!(
        released_before_ack,
        "close cannot acknowledge an active native writer"
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Serial session is closed"),
        "close cancels the remaining bytes"
    );
    assert!(count < MAX_WRITE_BYTES);
}

#[cfg(unix)]
#[tokio::test]
async fn read_side_disconnect_cancels_a_stalled_writer() {
    use super::{SerialManager, SerialSession, SerialWriteRequest, SerialWriter, MAX_WRITE_BYTES};
    use serialport::SerialPort;
    use std::{io::Read, sync::mpsc, time::Duration};
    let (mut peer, mut port) = serialport::TTYPort::pair().unwrap();
    peer.set_timeout(Duration::from_millis(50)).unwrap();
    port.set_timeout(Duration::from_millis(20)).unwrap();
    let writer = SerialWriter::new(super::prepare_serial_port(port).unwrap());
    let manager = SerialManager::default();
    let (control, _controls) = mpsc::channel();
    manager.sessions.lock().unwrap().insert(
        "fixture".into(),
        SerialSession {
            control,
            writer: writer.clone(),
        },
    );
    let writing = tokio::spawn(async move {
        manager
            .write(SerialWriteRequest {
                id: "fixture".into(),
                data: vec![0xa5; MAX_WRITE_BYTES],
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while writer.port.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let replace_writer = writer.clone();
    let mut disconnecting = tokio::task::spawn_blocking(move || replace_writer.replace(None));
    let cancelled = tokio::time::timeout(Duration::from_millis(300), &mut disconnecting)
        .await
        .map(|result| result.expect("disconnect worker must not panic"))
        .is_ok();
    // Release all task-owned resources even when running this test on the old code.
    writer.close();
    let draining = std::thread::spawn(move || {
        let mut buffer = [0; 8192];
        while matches!(peer.read(&mut buffer), Ok(n) if n > 0) {}
    });
    let result = writing.await.unwrap();
    if !cancelled {
        disconnecting.await.unwrap();
    }
    draining.join().unwrap();
    assert!(
        cancelled,
        "read-side disconnect must interrupt write polling before waiting for the lock"
    );
    assert!(result.is_err());
    assert!(writer.port.lock().unwrap().is_none());
}
