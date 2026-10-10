use std::{collections::VecDeque, future::poll_fn, task::Poll};

use russh::{client, ChannelMsg, ChannelReadHalf};
use tokio::sync::mpsc;

use super::SshControl;

#[cfg(test)]
mod tests;

pub(super) enum ShellEvent {
    Channel(ChannelMsg),
    ChannelClosed,
    Control(SshControl),
}

pub(super) async fn next_shell_event<H: client::Handler>(
    reader: &mut ChannelReadHalf,
    pending: &mut VecDeque<ChannelMsg>,
    controls: &mut mpsc::Receiver<SshControl>,
    handle: &mut client::Handle<H>,
    channel_closed: &mut bool,
) -> Option<ShellEvent> {
    loop {
        tokio::select! {
            // Observe already queued closure before dispatching controls.
            biased;
            message = async {
                if let Some(message) = pending.pop_front() {
                    Some(message)
                } else {
                    reader.wait().await
                }
            }, if !*channel_closed => match message {
                // Upstream SSHShellSession ends on EOF, not channel CLOSE or status.
                Some(ChannelMsg::Eof) => return None,
                Some(ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. }) => {},
                Some(ChannelMsg::Close) | None => {
                    *channel_closed = true;
                    return Some(ShellEvent::ChannelClosed);
                },
                Some(message) => return Some(ShellEvent::Channel(message)),
            },
            // Drain queued channel output before observing transport completion.
            // After CLOSE, disable the exhausted reader and retain tab controls.
            _ = &mut *handle, if *channel_closed => return None,
            control = controls.recv() => match control {
                // Reject resizing a removed shell while retaining transport controls.
                Some(SshControl::Resize(_, sender))
                    if *channel_closed => {
                        let _ = sender.send(Err(super::SshError::Closed));
                    },
                other => return other.map(ShellEvent::Control),
            },
        }
    }
}

pub(super) async fn run_control<F, E>(
    reader: &mut ChannelReadHalf,
    pending: &mut VecDeque<ChannelMsg>,
    channel_closed: &mut bool,
    input_task: &mut super::input::InputTask,
    operation: F,
    mut emit: E,
) -> bool
where
    F: std::future::Future<Output = bool>,
    E: FnMut(ChannelMsg) -> bool,
{
    tokio::pin!(operation);
    loop {
        tokio::select! {
            // Deliver a ready result before EOF can drop its reply.
            biased;
            result = &mut operation => return result,
            message = async {
                if let Some(message) = pending.pop_front() {
                    Some(message)
                } else {
                    reader.wait().await
                }
            }, if !*channel_closed => match message {
                // Upstream SSHShellSession ends on EOF, not channel CLOSE or status.
                Some(ChannelMsg::Eof) => {
                    input_task.stop().await;
                    // Completion may arrive while observing EOF or stopping input.
                    // A ready Tokio reply must not look pending because of task budget.
                    if let Poll::Ready(result) = tokio::task::unconstrained(poll_fn(|cx| {
                        Poll::Ready(operation.as_mut().poll(cx))
                    }))
                    .await
                    {
                        pending.push_front(ChannelMsg::Eof);
                        return result;
                    }
                    return false;
                }
                Some(ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. }) => {},
                // Retain the transport and pending operation until it finishes.
                Some(ChannelMsg::Close) | None => {
                    *channel_closed = true;
                    input_task.stop().await;
                },
                Some(message) => {
                    if !emit(message) {
                        input_task.stop().await;
                        // Deliver an available reply without continuing failed output.
                        let _ = tokio::task::unconstrained(poll_fn(|cx| {
                            Poll::Ready(operation.as_mut().poll(cx))
                        }))
                        .await;
                        return false;
                    }
                },
            },
        }
    }
}
