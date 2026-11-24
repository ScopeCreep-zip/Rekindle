//! The ring's `HeadAdvanced` hints (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3/§4.5), on both modes. A ring watcher that sees a member record it
//! watches advance subkey 1 (the beat, on a status transition only), 2 (the
//! op-log head), 3 (the frontier), 4 (the outbox) or 6 (the routes, plan
//! E3.4 S4) gossips a hint its pseudonym signs. A recipient drops a hint
//! from a non-member, then reads the writer-signed value; a frontier that
//! moved is the ring arm's frontier read and rebuild, a route goes into the
//! member route cache.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::member_record::head_advanced::{
    decode_head_advanced, HeadAdvanced, RELAYED_SUBKEYS,
};
use rekindle_codec::member_record::{beat::decode_presence_beat, BeatState, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_presence::community::Observed;
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, OUTBOX, ROUTES};
use rekindle_types::id::PseudonymKey;

use super::moderation::pseudonym;
use super::HostGovernance;
use crate::host::adapters::presence::view;

impl HostGovernance {
    /// The author whose member record is `record`, from the merged member
    /// set or, for the creator, the genesis cert.
    pub(super) fn author_of_record(
        &self,
        community_id: &str,
        record: &str,
    ) -> Option<PseudonymKey> {
        let state = self.ctx.communities.governance_state(community_id)?;
        state
            .members
            .iter()
            .find(|(_, m)| m.member_record == record)
            .map(|(p, _)| p.clone())
    }

    /// Relay a hint for each relayed subkey our watch saw `record` change:
    /// a beat only on a status transition (`beat_moved`), a frontier only
    /// when it follows the author's head advance (`frontier_follows_head`,
    /// never a republish, review S7m 83f7fc6b P2).
    pub(super) fn relay_head_advanced(
        &self,
        community_id: &str,
        record: &str,
        subkeys: &[u32],
        seqs: &[Option<u64>],
        beat_moved: bool,
        frontier_follows_head: bool,
    ) {
        let relayed: Vec<u32> = RELAYED_SUBKEYS
            .iter()
            .copied()
            .filter(|s| subkeys.contains(s))
            .collect();
        if relayed.is_empty() {
            return;
        }
        let (Some(author), Some(secret)) = (
            self.author_of_record(community_id, record),
            self.identity_secret_impl(),
        ) else {
            return;
        };
        let relayer = derive_community_pseudonym(&secret, community_id);
        for subkey in relayed {
            // Beat refreshes are not relayed, only transitions (§4.5).
            if subkey == BEAT && !beat_moved {
                continue;
            }
            // A frontier republish stays a ring-local signal (§4.5).
            if subkey == FRONTIER && !frontier_follows_head {
                continue;
            }
            let Some(Some(seq)) = seqs.get(subkey as usize).copied() else {
                continue;
            };
            let hint = HeadAdvanced {
                author: author.0,
                member_record: record.to_owned(),
                subkey,
                seq,
            };
            if subkey == FRONTIER {
                tracing::info!(community_id, record, seq, "ring relayed a frontier hint");
            }
            match hint.encode(community_id, &relayer) {
                Ok(hint) => self.send_to_mesh_impl(
                    community_id,
                    &CommunityEnvelope::control(ControlPayload::HeadAdvanced { hint }),
                ),
                Err(error) => tracing::debug!(community_id, %error, "head hint not encoded"),
            }
        }
    }

    /// Read `record`'s beat (subkey 1) through `lease`, forced through the
    /// hinted read: a ValueChange carries the value of its first subkey
    /// only (veilid-core 0.5.8 `veilid_state.rs:334-336`), so a beat named
    /// beside another subkey is not in the local copy. Its verified beat
    /// goes into the beat view (plan E3.4 S3) as delivered at `arrived_at`
    /// (now, for a change or hint; `None` for a cold read); the DHT seq read
    /// and what the view says it changed.
    pub(super) async fn read_member_beat(
        &self,
        community_id: &str,
        record: &str,
        lease: rekindle_records::lease::LeaseId,
        arrived_at: Option<u64>,
    ) -> Option<(u64, Observed)> {
        let author = self.author_of_record(community_id, record)?;
        let (seq, bytes) = self.read_hinted_value(lease, BEAT).await.ok().flatten()?;
        let at = RecordScope {
            community_id,
            member_record: record,
        };
        let beat = decode_presence_beat(&bytes, at, &author.0).ok()?;
        let observed = view::observe(&self.ctx, community_id, &author, &beat, arrived_at);
        if observed.state_changed {
            self.mesh_member_moved(community_id, &author).await;
        }
        Some((seq, observed))
    }

    /// What `member`'s beat (pseudonym hex) shows of them in
    /// `community_id` now: its state while present and live, `Offline`
    /// otherwise or when no beat of theirs reached us (plan E3.4 S3).
    #[must_use]
    pub fn member_beat_shown(&self, community_id: &str, member: &str) -> BeatState {
        view::shown(&self.ctx, community_id, member)
    }

    /// A ring change naming the beat (subkey 1): read it into the view;
    /// whether its state moved (relayed as a hint).
    pub(super) async fn ring_beat_moved(&self, community_id: &str, record: &str) -> bool {
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community_id)
                .and_then(|c| c.watch_ring.get(record).copied())
        };
        let Some(lease) = lease else {
            return false;
        };
        let now = Some(self.ctx.clock.now_ms());
        self.read_member_beat(community_id, record, lease, now)
            .await
            .is_some_and(|(_, observed)| observed.state_changed)
    }

    /// A gossiped hint, in §4.3's order: dropped unless its relayer is a
    /// member, then unless `admit` (the relayer's gossip rate limit) takes
    /// it, then unless it verifies under the relayer, names a relayed
    /// subkey (1, 2, 3, 4 or 6) and a member's own record. `true` when it passed
    /// (the transport relays it); the record is then read off the relay
    /// path, and the seq recorded is the one read, never the hint's
    /// (register S7l).
    pub fn on_head_advanced(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        hint: &[u8],
        admit: impl FnOnce(&str) -> bool,
    ) -> bool {
        let Some(relayer) = pseudonym(sender_pseudonym) else {
            return false;
        };
        if !self.is_key_reader(community_id, &relayer) {
            tracing::debug!(
                community_id,
                sender_pseudonym,
                "head hint from a non-member dropped"
            );
            return false;
        }
        if !admit(sender_pseudonym) {
            return false;
        }
        let Ok(hint) = decode_head_advanced(hint, community_id, &relayer.0) else {
            tracing::debug!(community_id, sender_pseudonym, "head hint did not verify");
            return false;
        };
        if !RELAYED_SUBKEYS.contains(&hint.subkey) {
            tracing::debug!(
                community_id,
                subkey = hint.subkey,
                "head hint names no relayed subkey"
            );
            return false;
        }
        if self.author_of_record(community_id, &hint.member_record)
            != Some(PseudonymKey(hint.author))
        {
            tracing::debug!(community_id, "head hint names no member's record");
            return false;
        }
        tracing::debug!(subkey = hint.subkey, "head hint received");
        // A round with no answer is retried on every `HeadAdvanced` (plan
        // E3.2 S7d-keys.8).
        self.ctx.communities.unlock_barriers().poke(community_id);
        let worth = self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .is_some_and(|c| {
                c.head_hints
                    .worth_reading(&hint.member_record, hint.subkey, hint.seq)
            });
        if worth {
            let host = self.clone();
            let community = community_id.to_owned();
            self.scope().spawn_or_drop("head hint read", async move {
                host.read_hinted(&community, hint).await;
            });
        }
        true
    }

    /// Read the hinted subkey from the DHT (writer-signed, forced through
    /// the hinted read, VU3) and record the seq that read returned; act
    /// only when it moved: a beat (1) goes into the beat view (plan E3.4
    /// S3), a route (6) into the member route cache (plan E3.4 S4); a
    /// frontier (3), the writer's commit point, is caught up and the
    /// community rebuilt when the read stored a head we lacked
    /// ([`HostGovernance::refresh_member_record`], which shares the ring
    /// change's (record, frontier seq) mark); an outbox (4) is diffed by
    /// the channel outbox reader (plan E3.3); an op-log head (2) or a beat
    /// (1) has no governance effect alone, so it is only recorded.
    async fn read_hinted(&self, community_id: &str, hint: HeadAdvanced) {
        let record = hint.member_record.as_str();
        if hint.subkey == FRONTIER {
            // The frontier is opened and read once, forced, by the
            // catch-up itself, and only by the trigger that claims it.
            self.refresh_member_record(community_id, record, hint.seq, false)
                .await;
            return;
        }
        let Ok(lease) = self.acquire_record(record, None, OpenIntent::Check).await else {
            return;
        };
        let read = if hint.subkey == BEAT {
            // A status transition a ring watcher relayed: into the view.
            let now = Some(self.ctx.clock.now_ms());
            let read = self
                .read_member_beat(community_id, record, lease, now)
                .await;
            if let Some((seq, observed)) = &read {
                tracing::debug!(
                    seq,
                    state_changed = observed.state_changed,
                    "hinted beat read and applied"
                );
            } else {
                tracing::debug!("hinted beat not read");
            }
            read.map(|(seq, _)| seq)
        } else if hint.subkey == ROUTES {
            self.read_member_routes(community_id, record, lease).await
        } else {
            self.read_hinted_value(lease, hint.subkey)
                .await
                .ok()
                .flatten()
                .map(|(seq, _)| seq)
        };
        self.release_record(lease).await;
        let Some(read) = read else {
            return;
        };
        {
            let mut map = self.ctx.communities.map().write();
            if let Some(c) = map.get_mut(community_id) {
                c.head_hints.record_read(record, hint.subkey, read);
            }
        }
        if hint.subkey == OUTBOX {
            self.read_channel_outbox(community_id, record).await;
        }
    }

    /// Forget the hint state of every record no current member owns
    /// (after a rebuild): a departed member's entries are never read again.
    pub(super) fn prune_head_hints(&self, community_id: &str) {
        let Some(state) = self.ctx.communities.governance_state(community_id) else {
            return;
        };
        // The creator's record too: the sweeps inspect it while it is in
        // the cert, member or not (`anti_entropy::member_records`).
        let records: std::collections::BTreeSet<String> = state
            .members
            .values()
            .map(|m| m.member_record.clone())
            .chain(
                rekindle_governance_runtime::deps::GovernanceRuntimeDeps::genesis_cert(
                    self,
                    community_id,
                )
                .map(|cert| cert.creator_member_record),
            )
            .collect();
        if let Some(c) = self.ctx.communities.map().write().get_mut(community_id) {
            c.head_hints.retain_records(&records);
        }
    }

    /// Diff the channel outbox (`M`/4) of the member whose record is
    /// `record` (plan E3.3: a hint or a ring change of subkey 4), never a
    /// governance rebuild.
    /// A departed author's outbox is read too (its record kept in
    /// `op_log_records`): an entry at or below its departure's head is kept,
    /// and a post it held while cut off reaches us only after the departure
    /// merged (netsim `a_leavers_withheld_last_write_is_kept_by_every_member`).
    pub(super) async fn read_channel_outbox(&self, community_id: &str, record: &str) {
        let departed = || {
            let state = self.ctx.communities.governance_state(community_id)?;
            (state.op_log_records.iter())
                .find(|(author, r)| *r == record && state.is_departed(author))
                .map(|(author, _)| author.clone())
        };
        let Some(author) = self
            .author_of_record(community_id, record)
            .or_else(departed)
        else {
            return;
        };
        crate::host::adapters::channel::HostChannel::new(&self.ctx)
            .read_author_outbox(community_id, &author, record, false)
            .await;
    }
}
