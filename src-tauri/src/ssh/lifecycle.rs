use std::collections::VecDeque;

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
