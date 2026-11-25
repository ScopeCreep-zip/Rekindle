//! The pipeline's events on the Host bus (plan E3.1 E2c): the echo of our
//! own post as a `ChannelMessageEvent::New`, and its delivery state as a
//! `ChannelMessageEvent::ChannelSendStatus`, which every frontend's windows
//! read on both modes; and a poll's change (`SocialEvent::PollChanged`).

use rekindle_channel::deps::SentChannelMessageEcho;
use rekindle_channel::event::ChannelEvent;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{ChannelMessageEvent, SocialEvent, SubscriptionEvent};

use super::HostChannel;
use crate::host::adapters::governance::community_profile::member_shown_name;

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
                forwarded_from_author: echo.forwarded_from_author.clone(),
                forwarded_from_name: self
                    .forward_name(&echo.community_id, echo.forwarded_from_author.as_deref()),
                automod_blurred: false,
                undecryptable: None,
            },
        ));
    }

    /// A forward's original `author` as `community_id` names a sender
    /// (plan E3.3): the member list's name, never the raw key. The echo,
    /// the ingest and history all name it here.
    pub(super) fn forward_name(&self, community_id: &str, author: Option<&str>) -> Option<String> {
        author.map(|a| member_shown_name(&self.ctx().communities, community_id, a))
    }

    pub(super) fn delivery_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        landed: bool,
    ) {
        let state = if landed {
            SendState::Sent
        } else {
            SendState::Failed
        };
        self.send_state_impl(community_id, channel_id, message_id, state, None);
    }

    /// Our post's `state` on the bus, with why it failed (`reason`, as the
    /// user reads it) when it did not go.
    pub(super) fn send_state_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        state: SendState,
        reason: Option<String>,
    ) {
        self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::ChannelSendStatus {
                community: community_id.to_string(),
                channel: channel_id.to_string(),
                message_id: message_id.to_string(),
                state,
                reason,
            },
        ));
    }

    /// A poll op of `poll_id` in `channel_id` landed on our outbox or was
    /// read from an author's: the poll's aggregate changed (plan E3.3 8c,
    /// `rekindle_channel::history::poll_states`), and a frontend re-reads
    /// the channel to show it. Nothing else tells it: a poll has no row
    /// and no gossip, and its create, vote and close are queued, so the
    /// read the frontend makes when its call returns precedes the write
    /// (walk 23 M53).
    pub(super) fn poll_changed(&self, community_id: &str, channel_id: &str, poll_id: &[u8; 16]) {
        self.ctx()
            .publish_event(SubscriptionEvent::Social(SocialEvent::PollChanged {
                community: community_id.to_owned(),
                channel: channel_id.to_owned(),
                poll_id: hex::encode(poll_id),
            }));
    }
}
