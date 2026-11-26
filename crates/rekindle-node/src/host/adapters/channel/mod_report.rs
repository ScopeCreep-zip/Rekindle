//! A moderator's report of a received channel post (plan E3.1, D26): the
//! ingest's `alert_moderators` verdict writes the post's `ModReport` and
//! raises `AutoModAlert` (`ingest`).

use rekindle_channel::error::ChannelError;
use rekindle_channel::ChannelMessagingDeps;
use rekindle_types::governance::{GovernanceEntry, ModReportTarget};
use rekindle_types::id::ChannelId;
use rekindle_types::permissions::MANAGE_MESSAGES;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::HostChannel;

impl HostChannel {
    /// An `alert_moderators` match (plan E3.1, D26): a moderator's ingest
    /// writes the post's `ModReport` and raises `AutoModAlert`. The report
    /// id is derived from the post, so every moderator's write is one
    /// queue row.
    pub(crate) async fn report_to_moderators(
        &self,
        community: &str,
        channel: &str,
        message_id: &str,
        reason: &str,
    ) {
        let moderator = self
            .require_channel_permission(community, Some(channel), MANAGE_MESSAGES)
            .is_ok();
        let (Some(channel_id), true) = (ChannelId::from_hex(channel), moderator) else {
            return;
        };
        let mut hasher = blake3::Hasher::new_derive_key("rekindle 2026-10 mod report id");
        hasher.update(community.as_bytes());
        hasher.update(&channel_id.0);
        hasher.update(message_id.as_bytes());
        let mut report_id = [0u8; 16];
        report_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        let held = self
            .governance_state(community)
            .is_some_and(|g| g.mod_reports.contains_key(&report_id));
        if !held {
            if let Err(e) = self
                .write_report(community, report_id, channel_id, message_id, reason)
                .await
            {
                tracing::warn!(error = %e, community, "moderator report not written");
            }
        }
        self.ctx()
            .publish_event(SubscriptionEvent::System(SystemEvent::AutoModAlert {
                community: community.to_owned(),
                channel: channel.to_owned(),
                message_id: message_id.to_owned(),
                rule_name: reason.to_owned(),
            }));
    }

    async fn write_report(
        &self,
        community: &str,
        report_id: [u8; 16],
        channel_id: ChannelId,
        message_id: &str,
        reason: &str,
    ) -> Result<(), ChannelError> {
        self.write_governance_entry(
            community,
            GovernanceEntry::ModReport {
                report_id,
                target: ModReportTarget::Message {
                    channel_id,
                    message_id: message_id.to_owned(),
                },
                reason: reason.to_owned(),
            },
        )
        .await
    }
}
