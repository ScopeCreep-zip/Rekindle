//! The pipeline's events on the Host bus (plan E3.1 E2c): the echo of our
//! own post as a `ChannelMessageEvent::New`, and its delivery state as a
//! `ChannelMessageEvent::ChannelSendStatus`, which every frontend's windows
//! read on both modes.

use rekindle_channel::deps::SentChannelMessageEcho;
use rekindle_channel::event::ChannelEvent;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::HostChannel;

impl HostChannel {
    pub(super) fn emit_event_impl(event: &ChannelEvent) {
        tracing::debug!(?event, "channel event");
    }

    pub(super) fn echo_impl(&self, echo: &SentChannelMessageEcho) {
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
        self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::ChannelSendStatus {
                community: community_id.to_string(),
                channel: channel_id.to_string(),
                message_id: message_id.to_string(),
                state: if landed {
                    SendState::Sent
                } else {
                    SendState::Failed
                },
            },
        ));
    }
}
