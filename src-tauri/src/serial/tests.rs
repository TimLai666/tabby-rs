use super::{write_serial_data, SerialOpenRequest};
use std::{
    collections::VecDeque,
    io::{self, ErrorKind, Write},
};

#[derive(Default)]
struct RecordingWriter {
    attempts: Vec<Vec<u8>>,
    bytes: Vec<u8>,
    answers: VecDeque<Result<usize, ErrorKind>>,
    flushes: usize,
}

impl Write for RecordingWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.attempts.push(data.to_vec());
        let length = self
            .answers
            .pop_front()
            .unwrap_or(Ok(data.len()))
            .map_err(io::Error::from)?
            .min(data.len());
        self.bytes.extend_from_slice(&data[..length]);
        Ok(length)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn slow_feed_matches_upstream_idle_stream_batching_without_flushing() {
    let data = [0, 127, 255, 0xe5, 0x8f, 0xb0, 13, 10];
    for slow in [false, true] {
        let mut port = RecordingWriter::default();
        write_serial_data(&mut port, &data, slow).unwrap();
        let expected: Vec<Vec<u8>> = if slow {
            vec![data[..1].to_vec(), data[1..].to_vec()]
        } else {
            vec![data.to_vec()]
        };
        assert_eq!(port.attempts, expected);
        assert_eq!(port.bytes, data);
        assert_eq!(port.flushes, 0);
    }
}

#[test]
fn partial_writes_preserve_all_bytes() {
    let mut port = RecordingWriter {
        answers: VecDeque::from([Ok(2), Ok(1)]),
        ..Default::default()
    };
    write_serial_data(&mut port, b"abcd", false).unwrap();
    assert_eq!(port.bytes, b"abcd");
    assert_eq!(
        port.attempts,
        [b"abcd".to_vec(), b"cd".to_vec(), b"d".to_vec()]
    );
}

#[test]
fn interrupted_writes_retry_the_same_bytes() {
    for slow in [false, true] {
        let mut port = RecordingWriter {
            answers: VecDeque::from([Err(ErrorKind::Interrupted)]),
            ..Default::default()
        };
        write_serial_data(&mut port, b"AB", slow).unwrap();
        assert_eq!(port.bytes, b"AB");
        assert_eq!(port.attempts[0], port.attempts[1]);
        assert_eq!(port.attempts.len(), if slow { 3 } else { 2 });
    }
}

#[test]
fn zero_write_reports_failure_without_retrying_forever() {
    for slow in [false, true] {
        let mut port = RecordingWriter {
            answers: VecDeque::from([Ok(0)]),
            ..Default::default()
        };
        assert_eq!(
            write_serial_data(&mut port, b"AB", slow)
                .unwrap_err()
                .kind(),
            ErrorKind::WriteZero
        );
        assert_eq!(port.attempts.len(), 1);
        assert!(port.bytes.is_empty());
    }
}

#[test]
fn failure_stops_the_remaining_bytes() {
    for slow in [false, true] {
        let mut port = RecordingWriter {
            answers: VecDeque::from([Ok(1), Err(ErrorKind::BrokenPipe)]),
            ..Default::default()
        };
        assert_eq!(
            write_serial_data(&mut port, b"ABC", slow)
                .unwrap_err()
                .kind(),
            ErrorKind::BrokenPipe
        );
        assert_eq!(port.attempts.len(), 2);
        assert_eq!(port.bytes, b"A");
        assert_eq!(port.flushes, 0);
    }
}

#[test]
fn empty_input_does_not_write() {
    for slow in [false, true] {
        let mut port = RecordingWriter::default();
        write_serial_data(&mut port, b"", slow).unwrap();
        assert!(port.attempts.is_empty());
        assert_eq!(port.flushes, 0);
    }
}

#[test]
fn slow_send_wire_option_preserves_legacy_requests() {
    let mut value = serde_json::json!({"profileId":"fixture", "connectionId":"fixture",
        "port":"/fixture", "baudRate":115200, "dataBits":8, "stopBits":1});
    let legacy: SerialOpenRequest = serde_json::from_value(value.clone()).unwrap();
    assert!(!legacy.slow_send);
    for enabled in [false, true] {
        value["slowSend"] = serde_json::json!(enabled);
        let request: SerialOpenRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.slow_send, enabled);
    }
    value["slowSend"] = serde_json::json!("true");
    assert!(serde_json::from_value::<SerialOpenRequest>(value).is_err());
}
