//! Channel typing decided in the Host (plan E3.4 S9).
//!
//! A window reports only whether its composer holds text
//! (`ChannelTyping { typing }`), like the DM path's draft report
//! (`rekindle_transport::operations::dm::typing`). The session state is the
//! same `rekindle_events::typing::TypingSessions` on the Host's friend
//! stores, keyed by `TypingContext::Channel`: the first report sends a start
//! at once and arms the idle timer, a report `TYPING_REFRESH_MS` (3 s) after
//! the last send re-sends it, and the stop (`ttlMs = 0`) goes out on a
//! cleared composer, on our message, or `TYPING_IDLE_MS` after the last
//! report. A start carries `ttlMs = CHANNEL_TYPING_TTL_MS` (6 s); the
//! receiver lapses it at arrival + min(ttl, 10 s)
//! (`operations::typing::channel_frame`), so a lost stop costs at most that.
//! Per-edit reports never become per-edit sends.
//!
//! The channel travels as its tag under the current text key of its scope
//! (`rekindle_secrets::channel_body::channel_tag`, the outbox entry's tag),
//! never in clear: only a holder of that key can tell which channel it is.

use std::time::Duration;

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_events::typing::{Input, Lapse, TypingSessions};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_secrets::channel_body::{channel_tag, CHANNEL_TAG_LEN};
use rekindle_transport::operations::friend::FriendHost;
use rekindle_transport::operations::typing;
use rekindle_types::channel_keys::current_key;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::TypingContext;

use super::HostGovernance;
use crate::host::adapters::friends::HostFriends;

/// The lifetime our channel typing start asks of its receivers (plan E3.4
/// S9): twice the re-send interval, so one lost re-send does not lapse it.
pub const CHANNEL_TYPING_TTL_MS: u32 = 6_000;

/// The typing context of `channel_id` in `community_id`.
fn context(community_id: &str, channel_id: &str) -> TypingContext {
    TypingContext::Channel {
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
    }
}

/// What one composer report puts on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draft {
    /// A new typing session: send the start and arm the idle timer.
    Started,
    /// The session's start is due again: re-send it.
    Refresh,
    /// The composer was cleared during a session: send the stop.
    Stopped,
    /// Nothing to send.
    Quiet,
}

/// The decision for one composer report at `now_ms`, over `sessions`.
fn decide(sessions: &mut TypingSessions, ctx: TypingContext, typing: bool, now_ms: u64) -> Draft {
    if !typing {
        return if sessions.end(&ctx) {
            Draft::Stopped
        } else {
            Draft::Quiet
        };
    }
    match sessions.input(ctx, now_ms) {
        Input::Started => Draft::Started,
        Input::Refresh => Draft::Refresh,
        Input::Quiet => Draft::Quiet,
    }
}

impl HostGovernance {
    /// The composer of `channel_id` changed: `typing` is whether it holds
    /// text.
    ///
    /// # Errors
    /// We hold no pseudonym or no text key here, or the frame did not reach
    /// the mesh.
    pub fn report_channel_draft(
        &self,
        community_id: &str,
        channel_id: &str,
        typing: bool,
    ) -> Result<(), GovernanceRuntimeError> {
        let friends = HostFriends::of(&self.ctx);
        let now = friends.clock().now_ms();
        let ctx = context(community_id, channel_id);
        let draft = decide(&mut friends.stores().typing_out.lock(), ctx, typing, now);
        match draft {
            Draft::Started => {
                self.watch_channel_idle(&friends, community_id, channel_id);
                self.send_typing_frame(community_id, channel_id, CHANNEL_TYPING_TTL_MS)
            }
            Draft::Refresh => {
                self.send_typing_frame(community_id, channel_id, CHANNEL_TYPING_TTL_MS)
            }
            Draft::Stopped => self.send_typing_frame(community_id, channel_id, 0),
            Draft::Quiet => Ok(()),
        }
    }

    /// Our message to `channel_id` went out: it ends our typing session there
    /// and sends the stop, with no report from the window.
    pub(crate) fn end_channel_typing(&self, community_id: &str, channel_id: &str) {
        let ended = (HostFriends::of(&self.ctx).stores().typing_out.lock())
            .end(&context(community_id, channel_id));
        if ended {
            if let Err(e) = self.send_typing_frame(community_id, channel_id, 0) {
                tracing::debug!(community = %community_id, error = %e, "channel typing stop not sent");
            }
        }
    }

    /// The gossip ingest's hook for a typing frame the meter admitted
    /// (`DaemonHandler::ingest_gossip`): `sender` is the envelope's verified
    /// signer and `pseudonym_key` the typist the frame names. A frame naming
    /// anyone but its signer is dropped, so no member can type as another.
    pub(crate) fn on_typing_frame(
        &self,
        community_id: &str,
        sender: &str,
        pseudonym_key: &str,
        tag: &[u8; CHANNEL_TAG_LEN],
        ttl_ms: u32,
    ) {
        if pseudonym_key != sender {
            tracing::debug!(community = %community_id, "typing frame names someone other than its signer");
            return;
        }
        self.on_channel_typing(community_id, sender, tag, ttl_ms);
    }

    /// A typing frame from `sender` (verified by the envelope's signature)
    /// in the channel tagged `tag`: lapsed or stopped by the Host, the
    /// typist named by [`Self::typist_name`].
    ///
    /// Only the current key of each channel's text scope is tried. A frame
    /// sealed under a key we do not hold as current (the sender's key
    /// rotated ahead of ours, or behind it) names no channel and is dropped
    /// silently: no event, no error. Typing is ephemeral, so the cost is at
    /// most one missed indicator until the next start under the key both
    /// sides hold; keeping old keys to name stale frames would widen what a
    /// departed member's key still reveals.
    pub(crate) fn on_channel_typing(
        &self,
        community_id: &str,
        sender: &str,
        tag: &[u8; CHANNEL_TAG_LEN],
        ttl_ms: u32,
    ) {
        let Some(channel) = self.channel_of_current_tag(community_id, tag) else {
            tracing::debug!(community = %community_id, "typing for no channel we can name");
            return;
        };
        let friends = HostFriends::of(&self.ctx);
        let name = self.typist_name(community_id, sender);
        typing::channel_frame(
            &friends,
            context(community_id, &channel),
            sender,
            name,
            ttl_ms,
        );
    }

    /// The Host's name for typist `sender` in `community_id`, as the member
    /// list names its rows (plan E3.2 F9-2): the name on their verified
    /// card, else their key's short fingerprint. Never empty, never a raw
    /// key, never a name the frame carries.
    pub(crate) fn typist_name(&self, community_id: &str, sender: &str) -> String {
        let pseudonym = PseudonymKey::from_hex_lossy(sender);
        let card_name = (self.ctx.communities).member_card_name(community_id, &pseudonym);
        super::community_profile::shown_name(card_name, &pseudonym)
    }

    /// `channel_id`'s tag under the current key of its text scope.
    fn current_channel_tag(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<[u8; 16], GovernanceRuntimeError> {
        let state = self.governance_state(community_id).ok_or_else(|| {
            GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned())
        })?;
        let id = *(state.channels.keys().find(|id| id.to_hex() == channel_id))
            .ok_or_else(|| GovernanceRuntimeError::OpRefused(format!("no channel {channel_id}")))?;
        let keys = self.keys();
        let scope = keys.scope_for_text(community_id, id);
        let (_, key) = current_key(&*keys, community_id, scope).ok_or_else(|| {
            GovernanceRuntimeError::OpRefused(format!("no text key for channel {channel_id}"))
        })?;
        Ok(channel_tag(&key, channel_id))
    }

    /// The channel whose tag under the current key of its text scope is
    /// `tag`.
    fn channel_of_current_tag(&self, community_id: &str, tag: &[u8; 16]) -> Option<String> {
        let state = self.governance_state(community_id)?;
        let keys = self.keys();
        state.channels.keys().find_map(|id| {
            let channel = id.to_hex();
            let scope = keys.scope_for_text(community_id, *id);
            let (_, key) = current_key(&*keys, community_id, scope)?;
            (channel_tag(&key, &channel) == *tag).then_some(channel)
        })
    }

    /// Send a start lasting `ttl_ms` (the stop when 0) for `channel_id`.
    fn send_typing_frame(
        &self,
        community_id: &str,
        channel_id: &str,
        ttl_ms: u32,
    ) -> Result<(), GovernanceRuntimeError> {
        let pseudonym_key = self
            .community_membership_impl(community_id)
            .and_then(|m| m.my_pseudonym_hex)
            .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_owned()))?;
        let channel_tag = self.current_channel_tag(community_id, channel_id)?;
        self.send_to_mesh(
            community_id,
            &CommunityEnvelope::TypingIndicator {
                channel_tag,
                ttl_ms,
                pseudonym_key,
            },
        )
    }

    /// Send the stop once the session in `channel_id` goes idle; a report
    /// moves the deadline, and an ended session or a lock ends the watch.
    fn watch_channel_idle(&self, friends: &HostFriends, community_id: &str, channel_id: &str) {
        let owned = friends.owned();
        let gov = self.clone();
        let (community, channel) = (community_id.to_owned(), channel_id.to_owned());
        friends
            .scope()
            .spawn_or_drop("channel typing idle stop", async move {
                let ctx = context(&community, &channel);
                loop {
                    let now = owned.clock().now_ms();
                    let lapse = owned.stores().typing_out.lock().idle(&ctx, now);
                    match lapse {
                        Lapse::Pending(deadline) => {
                            tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now)))
                                .await;
                        }
                        Lapse::Ended => {
                            if let Err(e) = gov.send_typing_frame(&community, &channel, 0) {
                                tracing::debug!(community = %community, error = %e, "channel typing stop not sent");
                            }
                            return;
                        }
                        Lapse::Gone => return,
                    }
                }
            });
    }
}

#[cfg(test)]
#[path = "channel_typing_tests.rs"]
mod tests;
