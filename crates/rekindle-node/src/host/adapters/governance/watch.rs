//! The governance arm of the Host's watch path (plan E3.1 C1, D4): a
//! ValueChange on a community's governance record (or one of its Plate Gate
//! segments' governance records) rebuilds that community alone, then
//! borrows every channel record the new state names that we do not hold.
//! Nothing polls a watched record: no timed inspect backs this up.

use rekindle_records::lease::CommunityLeases;
use rekindle_transport::broadcast::dht_writes;
use rekindle_types::id::ChannelId;

use super::HostGovernance;

impl HostGovernance {
    /// The community whose governance record is `record`.
    fn community_of_governance_record(&self, record: &str) -> Option<String> {
        self.ctx
            .communities
            .map()
            .read()
            .values()
            .find(|c| {
                c.governance_key.as_deref() == Some(record)
                    || c.governance_state
                        .as_ref()
                        .is_some_and(|g| g.segments.iter().any(|s| s.governance_key == record))
            })
            .map(|c| c.id.clone())
    }

    /// Handle a watched record's change when it is a governance record:
    /// `true` when it was one (rebuilt), `false` for any other record.
    pub async fn on_watched_change(&self, record: &str) -> bool {
        let Some(community) = self.community_of_governance_record(record) else {
            return false;
        };
        tracing::info!(%community, "governance record changed — rebuilding that community");
        rekindle_governance_runtime::rebuild_governance(self, &community).await;
        self.open_new_channel_records(&community).await;
        true
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
            match dht_writes::acquire_str(&transport, &record_key, writer.as_deref()).await {
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
        let merged = self
            .ctx
            .communities
            .hold_leases(community, leases, |l| dht_writes::key_of(&transport, l));
        for lease in merged.surplus {
            dht_writes::release(&transport, lease).await;
        }
        self.ctx.communities.note_channel_records(community, keys);
    }
}
