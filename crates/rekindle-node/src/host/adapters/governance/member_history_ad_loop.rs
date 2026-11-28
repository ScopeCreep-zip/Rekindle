//! The writer of our history ad (`M`/5) in every community, and the read of
//! the members' ads (plan E3.4 S7, "History ad moves to `M`/5"; ADR 0011),
//! one task per unlock on both modes, beside the member routes writer.
//!
//! **When.** Every [`AD_CHECK`] (the beat engine's shortest delay,
//! `rekindle_presence::community::BEAT_DELAY_MIN`, first at the unlock) the
//! writer computes each community's ranges from our stored messages and
//! writes `M`/5 only when the ranges or the community key generation
//! changed since its last write this unlock: a busy channel costs one
//! write per check at most, an idle one none. A rotation reseals the ad
//! under the new key at the next check, so the removed member, who keeps
//! the record key, cannot open it (W11.2).
//!
//! **Not written:** without a community key (an ad is always sealed), in a
//! community our own pseudonym departed (as the beat, ADR 0011), and while
//! our status hides presence (plan E3.4 "Invisible": a changing ad would
//! show members we are online).
//!
//! The registry row still carries its own ad until E3.4 S8a/S8b delete the
//! row (`rekindle-presence` `fit_history_ad`).

use std::collections::BTreeMap;
use std::time::Duration;

use rekindle_governance_runtime::member_card::MemberRecordRef;
use rekindle_governance_runtime::member_history_ad::{
    advertised, read_history_ad, write_history_ad,
};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_sync::history::{decode_member_history_ads, HistoryAd};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::HistoryRange;
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// How often the writer looks for a changed ad.
pub(crate) const AD_CHECK: Duration = rekindle_presence::community::BEAT_DELAY_MIN;

/// What our last ad in a community said: its ranges and key generation.
type Written = (Vec<HistoryRange>, u64);

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut checks = tokio::time::interval(AD_CHECK);
    let mut written: BTreeMap<String, Written> = BTreeMap::new();
    while stop.run_until_cancelled(checks.tick()).await.is_some() {
        if rekindle_presence::hides_presence(host.ctx.own_presence.status()) {
            continue;
        }
        let targets = host.sync_targets();
        written.retain(|community, _| targets.contains(community));
        for community_id in targets {
            if stop.is_cancelled() {
                return;
            }
            if host.own_departed(&community_id) {
                continue;
            }
            let Some(key) = host.community_key(&community_id) else {
                continue;
            };
            let ad = (
                advertised(host.held_ranges(&community_id).await),
                key.generation(),
            );
            if written.get(&community_id) == Some(&ad) {
                continue;
            }
            match host
                .publish_own_history_ad(&community_id, &ad.0, &key)
                .await
            {
                Ok(()) => {
                    written.insert(community_id, ad);
                }
                Err(error) => {
                    tracing::warn!(community = %community_id, %error, "history ad not written");
                }
            }
        }
    }
}

impl HostGovernance {
    /// The current `KeyScope::Community` key of `community_id`.
    pub(super) fn community_key(&self, community_id: &str) -> Option<MediaEncryptionKey> {
        (self.ctx.mek_cache.read())
            .current(community_id, KeyScope::Community)
            .cloned()
    }

    /// The channel ranges our stored messages of `community_id` span; none
    /// while locked or when the store fails. A stored channel id that is not
    /// a channel's 16 bytes in hex names no range.
    async fn held_ranges(&self, community_id: &str) -> Vec<HistoryRange> {
        let Some((owner, db)) = self.owner_and_db() else {
            return Vec::new();
        };
        let community = community_id.to_string();
        let rows = db
            .call(move |conn| {
                rekindle_db::repo::messages::channel::history_ranges(conn, &owner, &community)
            })
            .await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(community = %community_id, %error, "history ranges not read");
                return Vec::new();
            }
        };
        rows.into_iter()
            .filter_map(|(channel, oldest, newest)| {
                let id = hex::decode(&channel).ok()?.try_into().ok()?;
                Some(HistoryRange {
                    channel_id: ChannelId(id),
                    oldest_lamport: oldest,
                    newest_lamport: newest,
                })
            })
            .collect()
    }

    /// Write `ranges`, sealed under `key`, to our `M`/5 in `community_id`.
    ///
    /// # Errors
    /// We hold no member record or keys there, or the write failed.
    async fn publish_own_history_ad(
        &self,
        community_id: &str,
        ranges: &[HistoryRange],
        key: &MediaEncryptionKey,
    ) -> Result<(), String> {
        let keys = self.own_record_keys(community_id)?;
        let port = self.record_port().map_err(|e| e.to_string())?;
        write_history_ad(&*port, &keys.own(community_id), ranges, key, &*self.ctx.rng).await
    }

    /// The history ads of the members of `community_id` other than us, read
    /// from each one's `M`/5 (owner pinned to its `MemberJoined`) and opened
    /// with our current community key: the advertisers a joiner's bundle
    /// and a history request pick from (plan E3.4 S7, E3.5). A member whose
    /// ad is absent, unreadable or sealed under another generation is left
    /// out; no community key, no ads.
    pub async fn member_history_ads(
        &self,
        community_id: &str,
    ) -> Vec<(PseudonymKey, Vec<HistoryAd>)> {
        let (Some(state), Some(key)) = (
            self.governance_state(community_id),
            self.community_key(community_id),
        ) else {
            return Vec::new();
        };
        let Ok(port) = self.record_port() else {
            return Vec::new();
        };
        let me = self.my_pseudonym(community_id);
        let mut ads = Vec::new();
        for (pseudonym, member) in &state.members {
            if me.as_ref() == Some(pseudonym) || state.is_departed(pseudonym) {
                continue;
            }
            let at = MemberRecordRef {
                community_id,
                pseudonym,
                record: &member.member_record,
                owner: &member.member_record_owner,
            };
            let read = match read_history_ad(&*port, &at).await {
                Ok(Some(bytes)) => decode_member_history_ads(
                    &bytes,
                    &key,
                    rekindle_codec::member_record::RecordScope {
                        community_id,
                        member_record: &member.member_record,
                    },
                    &pseudonym.0,
                )
                .map_err(|e| e.to_string()),
                Ok(None) => continue,
                Err(error) => Err(error),
            };
            match read {
                Ok(ranges) => ads.push((pseudonym.clone(), ranges)),
                Err(error) => {
                    tracing::debug!(community = %community_id, %error, "history ad refused");
                }
            }
        }
        ads
    }
}
