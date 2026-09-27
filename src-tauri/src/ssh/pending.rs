use super::model::SshError;
use std::{
    collections::HashMap,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::{oneshot, watch};

#[derive(Clone, Default)]
pub(super) struct PendingConnections {
    entries: Arc<Mutex<HashMap<String, CancelSignal>>>,
}

impl PendingConnections {
    pub(super) fn begin(&self, id: &str) -> Result<PendingConnection, SshError> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if id.is_empty() || entries.contains_key(id) {
            return Err(SshError::InvalidRequest(
                "SSH connection id is empty or already pending".into(),
            ));
        }
        let signal = CancelSignal::default();
        entries.insert(id.into(), signal.clone());
        Ok(PendingConnection {
            id: id.into(),
            owner: self.clone(),
            signal,
        })
    }

    pub(super) fn signal(&self, id: &str) -> Option<CancelSignal> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
    }

    pub(super) fn cancel(&self, id: &str) {
        if let Some(signal) = self.signal(id) {
            signal.cancel();
        }
    }
}

pub(super) struct PendingConnection {
    id: String,
    owner: PendingConnections,
    pub(super) signal: CancelSignal,
}

impl Drop for PendingConnection {
    fn drop(&mut self) {
        let mut entries = self
            .owner
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if entries
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.sender, &self.signal.sender))
        {
            entries.remove(&self.id);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct CancelSignal {
    sender: Arc<watch::Sender<bool>>,
}

impl Default for CancelSignal {
    fn default() -> Self {
        Self {
            sender: Arc::new(watch::channel(false).0),
        }
    }
}

impl CancelSignal {
    pub(super) fn cancel(&self) {
        self.sender.send_replace(true);
    }

    async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }

    pub(super) async fn run<T>(
        &self,
        future: impl Future<Output = Result<T, SshError>>,
    ) -> Result<T, SshError> {
        tokio::select! {
            biased;
            _ = self.cancelled() => Err(SshError::Closed),
            result = future => result,
        }
    }

    pub(super) fn wrap<S>(&self, stream: S) -> CancellableStream<S> {
        let read = self.clone();
        let write = self.clone();
        CancellableStream {
            stream,
            // Split transports can read and write from different tasks. Each needs its own waker.
            read_cancelled: Some(Box::pin(async move { read.cancelled().await })),
            write_cancelled: Some(Box::pin(async move { write.cancelled().await })),
        }
    }
}

pub(super) struct CancellableStream<S> {
    stream: S,
    read_cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    write_cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

fn poll_cancellation(
    future: &mut Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    cx: &mut Context<'_>,
) -> io::Result<()> {
    let cancelled = match future.as_mut() {
        Some(future) => future.as_mut().poll(cx).is_ready(),
        None => true,
    };
    if cancelled {
        *future = None;
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "SSH connection cancelled",
        ))
    } else {
        Ok(())
    }
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for CancellableStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        poll_cancellation(&mut self.read_cancelled, cx)?;
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for CancellableStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        poll_cancellation(&mut self.write_cancelled, cx)?;
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        poll_cancellation(&mut self.write_cancelled, cx)?;
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Shutdown still reaches the underlying socket after cancellation.
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

pub(super) struct ReplyGuard<T> {
    id: String,
    waiters: Arc<Mutex<HashMap<String, oneshot::Sender<T>>>>,
}

impl<T> ReplyGuard<T> {
    pub(super) fn insert(
        waiters: &Arc<Mutex<HashMap<String, oneshot::Sender<T>>>>,
        id: String,
        sender: oneshot::Sender<T>,
    ) -> Self {
        waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), sender);
        Self {
            id,
            waiters: waiters.clone(),
        }
    }
}

impl<T> Drop for ReplyGuard<T> {
    fn drop(&mut self) {
        self.waiters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn transport_preserves_data_and_repeated_cancellation_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let signal = CancelSignal::default();
        let (stream, mut peer) = tokio::io::duplex(8);
        let mut stream = signal.wrap(stream);
        stream.write_all(b"out").await.unwrap();
        let mut bytes = [0; 3];
        peer.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"out");
        peer.write_all(b"in!").await.unwrap();
        stream.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"in!");
        signal.cancel();
        for _ in 0..2 {
            assert_eq!(
                stream.read(&mut bytes).await.unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
            assert_eq!(
                stream.write(b"x").await.unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
            assert_eq!(
                stream.flush().await.unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
        }
        stream.shutdown().await.unwrap();
        assert_eq!(peer.read(&mut bytes).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn cancellation_wakes_both_halves_of_a_blocked_transport() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let signal = CancelSignal::default();
        let (stream, _peer) = tokio::io::duplex(1);
        let (mut read, mut write) = tokio::io::split(signal.wrap(stream));
        let mut byte = [0];
        let read = read.read(&mut byte);
        let write = write.write_all(b"xx");
        tokio::pin!(read, write);
        std::future::poll_fn(|cx| {
            assert!(read.as_mut().poll(cx).is_pending());
            assert!(write.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        signal.cancel();
        let (read, write) = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(read, write)
        })
        .await
        .expect("both I/O waiters must wake");
        assert_eq!(
            read.unwrap_err().kind(),
            std::io::ErrorKind::ConnectionAborted
        );
        assert_eq!(
            write.unwrap_err().kind(),
            std::io::ErrorKind::ConnectionAborted
        );
    }

    #[tokio::test]
    async fn cancelled_key_exchange_closes_the_spawned_russh_transport() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (started, ready) = oneshot::channel();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.write_all(b"SSH-2.0-StalledKex\r\n").await.unwrap();
            let mut byte = [0];
            while stream.read_exact(&mut byte).await.is_ok() && byte[0] != b'\n' {}
            // A binary KEX packet proves connect_stream has spawned session.run.
            stream.read_exact(&mut byte).await.unwrap();
            started.send(()).unwrap();
            let mut tail = Vec::new();
            stream.read_to_end(&mut tail).await.unwrap();
        });
        struct Handler;
        impl russh::client::Handler for Handler {
            type Error = russh::Error;
        }
        let signal = CancelSignal::default();
        let stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let connect = signal.run(async {
            russh::client::connect_stream(
                Arc::new(russh::client::Config::default()),
                signal.wrap(stream),
                Handler,
            )
            .await
            .map_err(|_| SshError::Connection)
        });
        let cancel = async {
            ready.await.unwrap();
            signal.cancel();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(connect, cancel)
        })
        .await
        .unwrap();
        assert!(matches!(result, Err(SshError::Closed)));
        tokio::time::timeout(std::time::Duration::from_secs(1), peer)
            .await
            .expect("cancel must close the socket during key exchange")
            .unwrap();
    }

    #[tokio::test]
    async fn cancelling_a_prompt_removes_its_waiter_without_touching_another_prompt() {
        let pending = PendingConnections::default();
        let connection = pending.begin("a").unwrap();
        let waiters = Arc::new(Mutex::new(HashMap::new()));
        let (other_sender, mut other_receiver) = oneshot::channel::<bool>();
        let _other = ReplyGuard::insert(&waiters, "other".into(), other_sender);
        let (started, ready) = oneshot::channel();
        let work = connection.signal.run(async {
            let (sender, receiver) = oneshot::channel();
            let _reply = ReplyGuard::insert(&waiters, "a-prompt".into(), sender);
            started.send(()).unwrap();
            receiver.await.map_err(|_| SshError::Closed)
        });
        let cancel = async {
            ready.await.unwrap();
            pending.cancel("a");
        };
        let (result, ()) = tokio::join!(work, cancel);
        assert!(matches!(result, Err(SshError::Closed)));
        assert!(!waiters.lock().unwrap().contains_key("a-prompt"));
        assert!(matches!(
            other_receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        waiters
            .lock()
            .unwrap()
            .remove("other")
            .unwrap()
            .send(true)
            .unwrap();
        assert!(other_receiver.await.unwrap());
    }

    #[tokio::test]
    async fn early_cancellation_does_not_poll_the_connection() {
        let pending = PendingConnections::default();
        let connection = pending.begin("a").unwrap();
        pending.cancel("a");
        let polled = AtomicBool::new(false);
        let result = connection
            .signal
            .run(async {
                polled.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert!(matches!(result, Err(SshError::Closed)));
        assert!(!polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_drops_in_flight_resources_and_is_isolated() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let pending = PendingConnections::default();
        let a = pending.begin("a").unwrap();
        let b = pending.begin("b").unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let resource = Resource(dropped.clone());
        let (started, ready) = tokio::sync::oneshot::channel();
        let cancel = async {
            ready.await.unwrap();
            pending.cancel("a");
        };
        let work = a.signal.run(async move {
            let _resource = resource;
            started.send(()).unwrap();
            std::future::pending::<Result<(), SshError>>().await
        });
        let (result, ()) = tokio::join!(work, cancel);
        assert!(matches!(result, Err(SshError::Closed)));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(b.signal.run(async { Ok(42) }).await.unwrap(), 42);
    }

    #[tokio::test]
    async fn registrations_reject_overlap_and_release_without_tombstones() {
        let pending = PendingConnections::default();
        assert!(pending.begin("").is_err());
        pending.cancel("a");
        let first = pending.begin("a").unwrap();
        assert!(matches!(
            pending.begin("a"),
            Err(SshError::InvalidRequest(_))
        ));
        assert_eq!(first.signal.run(async { Ok(7) }).await.unwrap(), 7);
        let old_signal = first.signal.clone();
        drop(first);
        let second = pending.begin("a").unwrap();
        old_signal.cancel();
        assert_eq!(second.signal.run(async { Ok(8) }).await.unwrap(), 8);
        pending.cancel("a");
        pending.cancel("a");
        assert!(matches!(
            second.signal.run(async { Ok(()) }).await,
            Err(SshError::Closed)
        ));
        drop(second);
        assert!(pending.entries.lock().unwrap().is_empty());
    }
}
