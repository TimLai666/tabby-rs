// Exercise ready replies from the real control handler with queued shell EOF.
// This proves native reply delivery, not a remote SFTP/global-request exchange.
use super::*;
use crate::ssh::{input, SshError};

#[tokio::test]
async fn ready_native_sftp_reply_survives_queued_eof() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut f = Fixture::new().await;
        let (mut task, _input) = input::start(Arc::clone(&f._writer));
        let bytes = [0x00, 0xff, 0x80, b'x'];
        f.pending.push_back(ChannelMsg::Data {
            data: CryptoVec::from_slice(&bytes),
        });
        f.pending.push_back(ChannelMsg::Eof);
        let (reply, mut received) = oneshot::channel();
        let routes = Arc::new(std::sync::Mutex::new(Default::default()));
        let mut sftp = None;
        let mut emitted = 0;
        let keep_running = run_control(
            &mut f.reader,
            &mut f.pending,
            &mut f.channel_closed,
            &mut task,
            crate::ssh::handle_control(
                &mut f.handle,
                &f._writer,
                &mut sftp,
                "fixture",
                &routes,
                SshControl::SftpList { path: "/".into(), sender: reply },
            ),
            |_| { emitted += 1; true },
        ).await;
        let result = received.try_recv();
        let queued = f.pending.len();
        let closed = f.channel_closed;
        if keep_running {
            f.expect_data(&bytes).await;
            assert!(f.next().await.is_none(), "queued EOF still ends the shell");
        }
        drop(task);
        f.finish().await;
        assert!(keep_running, "a completed SFTP reply must be delivered before queued EOF");
        assert!(matches!(result, Ok(Err(SshError::InvalidRequest(ref message))) if message == "SFTP is not open"), "the caller must receive its actual SFTP error, not a dropped reply: {result:?}");
        assert_eq!(emitted, 0, "ready reply leaves output to the outer loop");
        assert_eq!(queued, 2);
        assert!(!closed);
    }).await.unwrap();
}

#[tokio::test]
async fn ready_native_close_reply_survives_queued_eof() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut f = Fixture::new().await;
        let (mut task, _input) = input::start(Arc::clone(&f._writer));
        f.pending.push_back(ChannelMsg::Eof);
        let (reply, mut received) = oneshot::channel();
        let routes = Arc::new(std::sync::Mutex::new(Default::default()));
        let mut sftp = None;
        let mut emitted = 0;
        let keep_running = run_control(
            &mut f.reader,
            &mut f.pending,
            &mut f.channel_closed,
            &mut task,
            crate::ssh::handle_control(
                &mut f.handle,
                &f._writer,
                &mut sftp,
                "fixture",
                &routes,
                SshControl::Close(reply),
            ),
            |_| { emitted += 1; true },
        ).await;
        let result = received.try_recv();
        let queued = f.pending.len();
        drop(task);
        f.finish().await;
        assert!(!keep_running, "completed local close ends the outer loop");
        assert!(matches!(result, Ok(Ok(()))), "false result alone cannot prove close completion; preserve the caller reply: {result:?}");
        assert_eq!(emitted, 0);
        assert_eq!(queued, 1, "local close does not consume queued EOF");
    }).await.unwrap();
}
