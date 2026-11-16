//! The governance arm of the Host's watch path (plan E3.2 S7, D4), on both
//! modes: a ValueChange on a member record our watch ring holds reads that
//! record's frontier, stores the heads we lack, and rebuilds that community
//! alone; then borrows every channel record the new state names that we do
//! not hold. This (with a `HeadAdvanced` hint) is the only governance
//! change trigger; the governance-record watch is gone.

use std::collections::BTreeSet;

use rekindle_governance_runtime::anti_entropy::{self, SeenFrontiers};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::lease::CommunityLeases;
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes;
use rekindle_types::id::ChannelId;

use super::HostGovernance;

impl HostGovernance {
    /// Handle a watched record's change when it is a ring member record:
    /// `true` when it was one (rebuilt), `false` for any other record.
    /// `subkeys` are the ones the change names; an empty list is a
    /// change we did not observe through our own watch, which is never
    /// relayed.
    pub async fn on_watched_change(&self, record: &str, subkeys: &[u32]) -> bool {
        let Some(community) = self.community_of_ring_record(record) else {
            return false;
        };
        // A community we were removed from is read-only history: a late
        // change opens nothing (plan E3.2 S9, R3-1).
        if self.ctx.communities.own_removal(&community).is_some() {
            return true;
        }
        tracing::info!(%community, record, "ring member record changed — rebuilding that community");
        self.refresh_member_record(&community, record).await;
        self.relay_head_advanced(&community, record, subkeys).await;
        true
    }

    /// Read `record`'s frontier, rebuild `community`, and borrow the
    /// channel records the new state names (a ring change or a hint).
    pub(super) async fn refresh_member_record(&self, community: &str, record: &str) {
        self.catch_up_record(community, record).await;
        rekindle_governance_runtime::rebuild_governance(self, community).await;
        self.open_new_channel_records(community).await;
    }

    /// Store the heads `record`'s frontier names that we do not hold.
    async fn catch_up_record(&self, community: &str, record: &str) {
        let known: Result<BTreeSet<_>, _> = self
            .governance_ops(community)
            .await
            .map(|held| held.iter().map(|op| op.hash).collect());
        let mut known = match known {
            Ok(known) => known,
            Err(error) => {
                tracing::warn!(community, %error, "ring change: op store not read");
                return;
            }
        };
        let mut seen = SeenFrontiers::new();
        if let Err(error) =
            anti_entropy::inspect_one(self, community, record, &mut seen, &mut known).await
        {
            tracing::warn!(community, record, %error, "ring change: heads not stored");
        }
    }

    /// Borrow every channel record the community's membership names that
    /// it does not hold yet, with our slot writer (channel records share the
    /// registry's slot seed), and hold the leases on the community.
    async fn open_new_channel_records(&self, community: &str) {
        let Ok(transport) = self.transport() else {
            return;
        };
        let named: Vec<(String, String)> = self
            .ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.communities.get(community))
            .map(|m| {
                m.channel_record_keys
                    .iter()
                    .map(|(channel, key)| (channel.clone(), key.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let (opened, writer) = self.ctx.communities.channel_records(community);
        let mut leases = CommunityLeases::default();
        let mut keys = Vec::new();
        for (channel_id, record_key) in named {
            if opened.contains(&record_key) {
                continue;
            }
            let Some(channel) = ChannelId::from_hex(&channel_id) else {
                continue;
            };
            match dht_writes::acquire_str(
                &transport,
                &record_key,
                writer.as_deref(),
                OpenIntent::Hold,
            )
            .await
            {
                Ok(lease) => {
                    leases.channels.insert(channel, lease);
                    keys.push(record_key);
                }
                Err(e) => tracing::debug!(
                    %community, channel_record = %record_key, error = %e,
                    "newly named channel record did not open"
                ),
            }
        }
        if keys.is_empty() {
            return;
        }
        self.ctx.communities.note_channel_records(community, keys);
        // The shell holds and watches its records itself (its leases move
        // at E3.4); the daemon's `SubscriptionManager` watches its own.
        if let Some(shell) = &self.ctx.shell {
            shell.community_records_ready(community, leases).await;
            return;
        }
        let merged = self
            .ctx
            .communities
            .hold_leases(community, leases, |l| dht_writes::key_of(&transport, l));
        for lease in merged.surplus {
            dht_writes::release(&transport, lease).await;
        }
    }
}
