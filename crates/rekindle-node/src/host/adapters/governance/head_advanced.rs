//! The ring's `HeadAdvanced` hints (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3/§4.5), on both modes. A ring watcher that sees a member record it
//! watches advance subkey 1 (the beat, on a status transition only), 2 (the
//! op-log head), 3 (the frontier) or 4 (the outbox) gossips a hint its
//! pseudonym signs; 6 is reserved until E3.4. A recipient drops a hint from
//! a non-member, then reads the writer-signed value; a frontier that moved
//! is the ring arm's frontier read and rebuild.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::member_record::head_advanced::{
    decode_head_advanced, HeadAdvanced, RELAYED_SUBKEYS,
};
use rekindle_codec::member_record::{beat::decode_presence_beat, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::port::OpenIntent;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, OUTBOX};
use rekindle_types::id::PseudonymKey;

use super::moderation::pseudonym;
use super::HostGovernance;

impl HostGovernance {
    /// The author whose member record is `record`, from the merged member
    /// set or, for the creator, the genesis cert.
    fn author_of_record(&self, community_id: &str, record: &str) -> Option<PseudonymKey> {
        let state = self.ctx.communities.governance_state(community_id)?;
        state
            .members
            .iter()
            .find(|(_, m)| m.member_record == record)
            .map(|(p, _)| p.clone())
    }

    /// Relay a hint for each relayed subkey our watch saw `record` change.
    pub(super) async fn relay_head_advanced(
        &self,
        community_id: &str,
        record: &str,
        subkeys: &[u32],
        seqs: &[Option<u64>],
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
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community_id)
                .and_then(|c| c.watch_ring.get(record).copied())
        };
        let Some(lease) = lease else {
            return;
        };
        let relayer = derive_community_pseudonym(&secret, community_id);
        for subkey in relayed {
            if subkey == BEAT
                && !self
                    .beat_status_changed(community_id, record, &author, lease)
                    .await
            {
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
            match hint.encode(community_id, &relayer) {
                Ok(hint) => self.send_to_mesh_impl(
                    community_id,
                    &CommunityEnvelope::Control(ControlPayload::HeadAdvanced { hint }),
                ),
                Err(error) => tracing::debug!(community_id, %error, "head hint not encoded"),
            }
        }
    }

    /// Did `record`'s beat change status since the last we saw? Beat
    /// refreshes are not relayed, only transitions (§4.5).
    async fn beat_status_changed(
        &self,
        community_id: &str,
        record: &str,
        author: &PseudonymKey,
        lease: rekindle_records::lease::LeaseId,
    ) -> bool {
        let Ok(Some(bytes)) = self.get_dht_value(lease, BEAT, false).await else {
            return false;
        };
        let at = RecordScope {
            community_id,
            member_record: record,
        };
        let Ok(beat) = decode_presence_beat(&bytes, at, &author.0) else {
            return false;
        };
        let mut map = self.ctx.communities.map().write();
        let Some(community) = map.get_mut(community_id) else {
            return false;
        };
        community
            .ring_beat_status
            .insert(record.to_owned(), beat.status)
            != Some(beat.status)
    }

    /// A gossiped hint, in §4.3's order: dropped unless its relayer is a
    /// member, then unless `admit` (the relayer's gossip rate limit) takes
    /// it, then unless it verifies under the relayer, names a relayed
    /// subkey (1, 2, 3 or 4) and a member's own record. `true` when it passed
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
    /// only when it moved: a
    /// frontier (3), the writer's commit point, is caught up and the
    /// community rebuilt when the read stored a head we lacked
    /// ([`HostGovernance::refresh_member_record`], which shares the ring
    /// change's (record, frontier seq) mark); an outbox (4) is diffed by
    /// the channel outbox reader (plan E3.3); an op-log head (2) or a beat
    /// (1) has no governance effect alone, so it is only recorded.
    async fn read_hinted(&self, community_id: &str, hint: HeadAdvanced) {
        let record = hint.member_record.as_str();
        let Ok(lease) = self.acquire_record(record, None, OpenIntent::Check).await else {
            return;
        };
        if hint.subkey == FRONTIER {
            // The frontier is read once, forced, by the catch-up itself.
            self.release_record(lease).await;
            self.refresh_member_record(community_id, record).await;
            return;
        }
        let read = self
            .read_hinted_value(lease, hint.subkey)
            .await
            .ok()
            .flatten()
            .map(|(seq, _)| seq);
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

#[cfg(test)]
mod tests {
    use crate::host::community::state::HeadHints;

    /// One member's `u64::MAX` hint must not mute a record (register S7l,
    /// `evidence/slot-integrity.md` §4.3): the seq kept is the one read
    /// from the DHT, so the record's next real advance is still read.
    #[test]
    fn a_hint_at_u64_max_does_not_mute_the_record() {
        let mut hints = HeadHints::default();
        assert!(hints.worth_reading("M", 2, u64::MAX));
        assert!(hints.record_read("M", 2, 7), "the DHT held seq 7");
        assert!(
            hints.worth_reading("M", 2, 8),
            "the record's real seq 8 is still read"
        );
        assert!(!hints.worth_reading("M", 2, 7), "seq 7 was read");
        assert!(hints.record_read("M", 2, 8));
        assert!(
            !hints.record_read("M", 2, 8),
            "an unmoved read is not acted on"
        );
    }

    /// A subkey's first value is at seq 0 (Veilid `ValueSeqNum::next`
    /// from NONE): a hint at seq 0 for a subkey never read is worth a
    /// read, and that read is acted on, so a re-joiner's first post on
    /// its fresh member record reaches the members by its own hint
    /// (netsim `a_rejoin_missed_by_gossip_converges_by_the_sweep`).
    #[test]
    fn a_first_value_at_seq_0_is_read() {
        let mut hints = HeadHints::default();
        assert!(hints.worth_reading("M", 4, 0), "subkey 4 was never read");
        assert!(hints.record_read("M", 4, 0), "the first read is acted on");
        assert!(!hints.worth_reading("M", 4, 0), "seq 0 was read");
        assert!(!hints.record_read("M", 4, 0));
        assert!(hints.worth_reading("M", 4, 1));
    }
}
