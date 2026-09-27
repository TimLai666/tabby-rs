use std::sync::Arc;

use russh::{client, ChannelWriteHalf};
use tokio::{
    io::AsyncWriteExt,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use super::SshError;

pub(super) type ShellWrite = (Vec<u8>, oneshot::Sender<Result<(), SshError>>);

pub(super) struct InputTask(Option<JoinHandle<()>>);

impl InputTask {
    fn cancel(&self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }

    pub(super) async fn stop(&mut self) {
        // Join before sending CLOSE so the writer cannot enqueue data afterward.
        if let Some(task) = self.0.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for InputTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) fn start(
    writer: Arc<ChannelWriteHalf<client::Msg>>,
) -> (InputTask, mpsc::Sender<ShellWrite>) {
    let (sender, mut receiver) = mpsc::channel::<ShellWrite>(32);
    let task = tokio::spawn(async move {
        while let Some((data, reply)) = receiver.recv().await {
            let mut sink = writer.make_writer();
            let result = match sink.write_all(&data).await {
                Ok(()) => sink.flush().await.map_err(|_| SshError::Closed),
                Err(_) => Err(SshError::Closed),
            };
            let failed = result.is_err();
            let _ = reply.send(result);
            if failed {
                break;
            }
        }
    });
    (InputTask(Some(task)), sender)
}
