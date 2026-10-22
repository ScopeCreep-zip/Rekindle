//! A community's history catch-up (plan E3.1 C2a), on both hosts: after a
//! settle, read the members' presence rows from the registry, take the
//! history ranges each verified row advertises under the current MEK, and
//! ask the best peer per channel for what we lack (`SyncRequest`). It runs
//! on the community's scope, so leaving or locking stops it.

use std::collections::HashMap;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_sync::history::{select_best_peer, select_deepest_peer, HistoryAd};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::presence::{EncryptedHistoryRanges, HistoryRange, MemberPresence};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// How long a catch-up waits for the community's records and presence to
/// settle before it reads the registry.
const CATCHUP_SETTLE: std::time::Duration = std::time::Duration::from_secs(15);

/// One peer's advertisement: its pseudonym, route blob and history ranges.
type PeerAds = (String, Vec<u8>, Vec<HistoryAd>);

impl HostGovernance {
    /// Schedule `community_id`'s history catch-up on its scope.
    pub(super) fn spawn_history_catchup_impl(&self, community_id: &str) {
        let host = self.clone();
        let id = community_id.to_string();
        self.ctx
            .community_scope(community_id)
            .spawn_with_token_or_drop("community history catch-up", move |stop| async move {
                if stop
                    .run_until_cancelled(tokio::time::sleep(CATCHUP_SETTLE))
                    .await
                    .is_none()
                {
                    return;
                }
                if let Err(error) = host.request_history_catchup(&id, &stop).await {
                    tracing::debug!(community = %id, %error, "history catch-up skipped");
                }
            });
    }

    /// Ask, per channel, the peer whose ads reach furthest back past what
    /// we hold; stops before the next network call once `stop` fires.
    async fn request_history_catchup(
        &self,
        community_id: &str,
        stop: &CancellationToken,
    ) -> Result<(), String> {
        let (registry_key, channel_ids) = {
            let map = self.ctx.communities.map().read();
            let community = map.get(community_id).ok_or("community not held")?;
            let registry = community
                .member_registry_key
                .clone()
                .ok_or("missing registry key")?;
            let channels: Vec<String> = community.channels.iter().map(|c| c.id.clone()).collect();
            (registry, channels)
        };
        let local_oldest = self.local_oldest_lamports(community_id).await?;
        if stop.is_cancelled() {
            return Ok(());
        }
        let peer_ads = self.read_peer_ads(community_id, &registry_key).await?;
        for channel_id in &channel_ids {
            // Each request is an `app_call`: stop before the next one (C4.L1).
            if stop.is_cancelled() {
                return Ok(());
            }
            let current_oldest = local_oldest.get(channel_id).copied().unwrap_or(0);
            let Some(route_blob) = best_peer_for(&peer_ads, channel_id, current_oldest) else {
                continue;
            };
            let request = CommunityEnvelope::Control(ControlPayload::SyncRequest {
                channel_id: channel_id.clone(),
                since_timestamp: 0,
            });
            let bytes =
                serde_json::to_vec(&request).map_err(|e| format!("sync request serialize: {e}"))?;
            self.app_call_peer_impl(route_blob, bytes)
                .await
                .map_err(|e| format!("sync request failed: {e}"))?;
        }
        Ok(())
    }

    /// The registry is the membership index: one read of the rows that hold
    /// a value (plan C7.12); each row's signature is verified before its
    /// ranges are trusted (architecture §26 W26).
    async fn read_peer_ads(
        &self,
        community_id: &str,
        registry_key: &str,
    ) -> Result<Vec<PeerAds>, String> {
        let node = self.transport().map_err(|e| e.to_string())?;
        let pool = node.require_records().map_err(|e| e.to_string())?;
        let all_slots: Vec<u32> = (0..rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT)
            .filter_map(|slot| u32::try_from(slot).ok())
            .collect();
        let lease = pool
            .acquire(registry_key, None)
            .await
            .map_err(|e| format!("registry open failed: {e}"))?;
        let rows = pool.read_changed(lease, &all_slots).await;
        pool.release(lease).await;
        let rows = rows.map_err(|e| format!("registry read failed: {e}"))?;
        Ok(rows
            .into_iter()
            .filter_map(|(_, value)| self.peer_ad(community_id, &value.data))
            .collect())
    }

    /// One verified presence row's advertisement, if it carries ranges we
    /// can read.
    fn peer_ad(&self, community_id: &str, data: &[u8]) -> Option<PeerAds> {
        if data.is_empty() {
            return None;
        }
        let presence = serde_json::from_slice::<MemberPresence>(data).ok()?;
        let sig: [u8; 64] = presence.signature.as_slice().try_into().ok()?;
        rekindle_secrets::derive::verify_pseudonym_signature(
            &presence.pseudonym_key.0,
            &presence.signing_bytes(),
            &sig,
        )
        .ok()?;
        if presence.route_blob.is_empty() {
            return None;
        }
        // W11.2: the ranges are sealed under the community MEK; a member
        // without the current generation skips this peer's ads.
        let ranges =
            self.decrypt_history_ranges(community_id, presence.history_ranges_encrypted.as_ref()?)?;
        if ranges.is_empty() {
            return None;
        }
        let ads = ranges
            .iter()
            .map(|range| HistoryAd {
                channel_id: hex::encode(range.channel_id.0),
                oldest_lamport: range.oldest_lamport,
                newest_lamport: range.newest_lamport,
            })
            .collect();
        Some((
            hex::encode(presence.pseudonym_key.0),
            presence.route_blob,
            ads,
        ))
    }

    /// Current generation only: a member the last rotation removed holds
    /// only the replaced key, and its presence must not read.
    fn decrypt_history_ranges(
        &self,
        community_id: &str,
        encrypted: &EncryptedHistoryRanges,
    ) -> Option<Vec<HistoryRange>> {
        let plaintext = {
            let cache = self.ctx.mek_cache.read();
            let mek = cache
                .current(community_id, KeyScope::Community)
                .filter(|mek| mek.generation() == encrypted.mek_generation)?;
            mek.decrypt(&encrypted.ciphertext).ok()?
        };
        serde_json::from_slice(&plaintext).ok()
    }

    /// The oldest Lamport we hold per channel of `community_id`.
    async fn local_oldest_lamports(
        &self,
        community_id: &str,
    ) -> Result<HashMap<String, u64>, String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity unlocked".to_string())?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let community = community_id.to_string();
        db.call(move |conn| {
            rekindle_db::repo::channel_bundle::oldest_lamport_per_channel(conn, &owner, &community)
        })
        .await
        .map_err(|e| e.to_string())
    }
}

/// The route blob of the peer to ask for `channel_id`'s history, when one
/// reaches further back than `current_oldest`.
fn best_peer_for<'a>(
    peer_ads: &'a [PeerAds],
    channel_id: &str,
    current_oldest: u64,
) -> Option<&'a [u8]> {
    let candidates: Vec<_> = peer_ads
        .iter()
        .enumerate()
        .filter_map(|(idx, (_, _, ads))| {
            ads.iter()
                .find(|ad| ad.channel_id == channel_id)
                .map(|ad| (idx, ad))
        })
        .collect();
    let needed = current_oldest.saturating_sub(1);
    let selected = if needed == 0 {
        select_deepest_peer(&candidates)
    } else {
        select_best_peer(&candidates, needed)
    }?;
    let (_, route_blob, ads) = peer_ads.get(selected)?;
    let best = ads.iter().find(|ad| ad.channel_id == channel_id)?;
    if best.oldest_lamport >= current_oldest && current_oldest != 0 {
        return None;
    }
    Some(route_blob)
}
