use std::{collections::VecDeque, future::poll_fn, task::Poll};

use russh::{ChannelMsg, ChannelReadHalf};
use tokio::sync::mpsc;

use super::SshControl;

#[cfg(test)]
mod tests;

pub(super) enum ShellEvent {
    Channel(ChannelMsg),
    Control(SshControl),
}

pub(super) async fn next_shell_event(
    reader: &mut ChannelReadHalf,
    pending: &mut VecDeque<ChannelMsg>,
    controls: &mut mpsc::Receiver<SshControl>,
) -> Option<ShellEvent> {
    loop {
        tokio::select! {
            // Observe already queued output and closure before dispatching controls.
            biased;
            message = async {
                if let Some(message) = pending.pop_front() {
                    Some(message)
                } else {
                    reader.wait().await
                }
            } => match message {
                // Upstream SSHShellSession (9e7a251b) ends on EOF or channel CLOSE,
                // since servers may send CLOSE without EOF. Status alone does not end it.
                // A closed reader (transport disconnection) ends it after queued output.
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => return None,
                Some(ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. }) => {},
                Some(message) => return Some(ShellEvent::Channel(message)),
            },
            control = controls.recv() => return control.map(ShellEvent::Control),
        }
    }
}

pub(super) async fn run_control<F, E>(
    reader: &mut ChannelReadHalf,
    pending: &mut VecDeque<ChannelMsg>,
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
            } => match message {
                // Upstream SSHShellSession (9e7a251b) ends on EOF or channel CLOSE.
                // A closed reader (transport disconnection) ends it as well.
                terminal @ (Some(ChannelMsg::Eof | ChannelMsg::Close) | None) => {
                    input_task.stop().await;
                    // Completion may arrive while observing the end or stopping input.
                    // A ready Tokio reply must not look pending because of task budget.
                    if let Poll::Ready(result) = tokio::task::unconstrained(poll_fn(|cx| {
                        Poll::Ready(operation.as_mut().poll(cx))
                    }))
                    .await
                    {
                        // Retain the end for the outer loop; a closed reader repeats None.
                        if let Some(message) = terminal {
                            pending.push_front(message);
                        }
                        return result;
                    }
                    return false;
                }
                Some(ChannelMsg::ExitStatus { .. } | ChannelMsg::ExitSignal { .. }) => {},
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
