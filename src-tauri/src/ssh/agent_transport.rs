use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use russh::keys::agent::client::AgentStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

// Give russh's generic signer a concrete stream type. A bare boxed AgentStream
// causes its authentication future to fail the async-trait Send lifetime bound.
pub(super) struct AgentTransport(Box<dyn AgentStream + Send + Unpin + 'static>);

impl AgentTransport {
    pub(super) fn new(stream: Box<dyn AgentStream + Send + Unpin + 'static>) -> Self {
        Self(stream)
    }
}

impl AsyncRead for AgentTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for AgentTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.0).poll_write_vectored(cx, bufs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn transfers_binary_data_with_backpressure_and_eof() {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let (stream, mut peer) = tokio::io::duplex(1);
            let mut transport = AgentTransport::new(Box::new(stream));
            let bytes = [0, 255, 0xe5, 0x8f, 0xb0];
            let (sent, received) = tokio::join!(
                async {
                    transport.write_all(&bytes).await.unwrap();
                    transport.flush().await.unwrap();
                    transport.shutdown().await.unwrap();
                    let mut reply = Vec::new();
                    transport.read_to_end(&mut reply).await.unwrap();
                    reply
                },
                async {
                    let mut received = Vec::new();
                    peer.read_to_end(&mut received).await.unwrap();
                    peer.write_all(&received).await.unwrap();
                    peer.shutdown().await.unwrap();
                    received
                }
            );
            assert_eq!(sent, bytes);
            assert_eq!(received, bytes);
        })
        .await
        .expect("transport must wake through partial reads and writes");
    }

    #[tokio::test]
    async fn propagates_peer_closure_and_write_errors() {
        let (stream, peer) = tokio::io::duplex(1);
        let mut transport = AgentTransport::new(Box::new(stream));
        drop(peer);
        assert_eq!(transport.read(&mut [0; 1]).await.unwrap(), 0);
        assert_eq!(
            transport.write_all(b"x").await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
