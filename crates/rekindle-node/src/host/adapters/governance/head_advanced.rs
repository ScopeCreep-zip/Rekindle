//! The ring's `HeadAdvanced` hints (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3/§4.5), on both modes. A ring watcher that sees a member record it
//! watches advance subkey 1 (the beat, on a status transition only), 2 (the
//! op-log head) or 4 (the outbox) gossips a hint its pseudonym signs; 6 is
//! reserved until E3.4. A recipient drops a hint from a non-member, then
//! reads the writer-signed value: the ring arm's frontier read and rebuild.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::member_record::head_advanced::{
    decode_head_advanced, HeadAdvanced, RELAYED_SUBKEYS,
};
use rekindle_codec::member_record::{beat::decode_presence_beat, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_types::dht_layout::member_record::BEAT;
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
        let Ok(seqs) = self.inspect_dht_record_update_get_seqs(lease).await else {
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

    /// A gossiped hint: dropped unless its relayer is a member, it verifies
    /// under the relayer, and it names a member's own record; then the
    /// writer-signed value is read and the community rebuilt, once per
    /// (record, subkey, seq).
    pub async fn on_head_advanced(&self, community_id: &str, sender_pseudonym: &str, hint: &[u8]) {
        let Some(relayer) = pseudonym(sender_pseudonym) else {
            return;
        };
        if !self.is_key_reader(community_id, &relayer) {
            tracing::debug!(
                community_id,
                sender_pseudonym,
                "head hint from a non-member dropped"
            );
            return;
        }
        let Ok(hint) = decode_head_advanced(hint, community_id, &relayer.0) else {
            tracing::debug!(community_id, sender_pseudonym, "head hint did not verify");
            return;
        };
        if self.author_of_record(community_id, &hint.member_record)
            != Some(PseudonymKey(hint.author))
        {
            tracing::debug!(community_id, "head hint names no member's record");
            return;
        }
        let fresh = {
            let mut map = self.ctx.communities.map().write();
            let Some(community) = map.get_mut(community_id) else {
                return;
            };
            let seen = community
                .head_hints
                .entry((hint.member_record.clone(), hint.subkey))
                .or_insert(0);
            let fresh = hint.seq > *seen;
            *seen = (*seen).max(hint.seq);
            fresh
        };
        if fresh {
            self.refresh_member_record(community_id, &hint.member_record)
                .await;
        }
    }
}
