//! The pipeline's events: the echo of our own post and its delivery state.
//! The GUI's windows still read its legacy shapes, so the shell gets them
//! through [`ShellHooks`](crate::host::shell_hooks::ShellHooks) until E2c;
//! `rekindled` publishes the echo on the Host bus, and a post's delivery
//! state is its outbox row's.

use rekindle_channel::deps::SentChannelMessageEcho;
use rekindle_channel::event::ChannelEvent;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::HostChannel;

impl HostChannel {
    pub(super) fn emit_event_impl(event: &ChannelEvent) {
        tracing::debug!(?event, "channel event");
    }

    pub(super) fn echo_impl(&self, echo: &SentChannelMessageEcho) {
        if let Some(shell) = &self.ctx().shell {
            shell.channel_echo(echo);
            return;
        }
        self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New {
                community: echo.community_id.clone(),
                channel: echo.channel_id.clone(),
                message_id: echo.message_id.clone(),
                sender_pseudonym: echo.sender_pseudonym.clone(),
                sequence: 0,
                timestamp: echo.timestamp_ms,
                body: Some(echo.body.clone()),
                reply_to_message_id: None,
            },
        ));
    }

    pub(super) fn delivery_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        landed: bool,
    ) {
        match &self.ctx().shell {
            Some(shell) => shell.channel_post_settled(community_id, channel_id, message_id, landed),
            None => tracing::info!(community_id, channel_id, message_id, landed, "post settled"),
        }
    }
}
