//! Our channel outbox (`M`/4, plan E3.3) as the governance adapter knows
//! it: who writes it (our member record, its owner `K_M(p)` from the vault,
//! our community pseudonym) and the author's own `HeadAdvanced` for subkey
//! 4 after a drain lands, so readers diff it after one DHT set and one
//! gossip round (the plan's fast path), not after a watch and a relay.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::member_record::head_advanced::HeadAdvanced;
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::OUTBOX;

use super::HostGovernance;

/// Who appends to our channel outbox in one community.
pub(crate) struct OwnOutbox {
    pub member_record: String,
    pub owner: zeroize::Zeroizing<String>,
    pub author: SigningKey,
}

impl HostGovernance {
    /// Our outbox writer of `community_id`.
    ///
    /// # Errors
    /// No identity is unlocked, or we hold no member record there.
    pub(crate) fn own_outbox(&self, community_id: &str) -> Result<OwnOutbox, String> {
        let secret = self
            .identity_secret_impl()
            .ok_or("no identity is unlocked")?;
        let member_record = self
            .own_member_record(community_id)
            .ok_or("we hold no member record in this community")?;
        Ok(OwnOutbox {
            member_record,
            owner: self.member_record_writer(community_id)?,
            author: derive_community_pseudonym(&secret, community_id),
        })
    }

    /// A drain landed on our outbox: gossip our own hint for subkey 4 at
    /// the seq of the head value we wrote.
    pub(crate) async fn hint_own_outbox(&self, community_id: &str, own: &OwnOutbox) {
        let pool = self
            .transport()
            .map_err(|e| e.to_string())
            .and_then(|t| t.require_records().map_err(|e| e.to_string()));
        let Ok(pool) = pool else {
            return;
        };
        let seq = match pool.read_once(&own.member_record, OUTBOX, false).await {
            Ok(Some(value)) => u64::from(value.seq),
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(community_id, %error, "own outbox head not read for its hint");
                return;
            }
        };
        let hint = HeadAdvanced {
            author: own.author.verifying_key().to_bytes(),
            member_record: own.member_record.clone(),
            subkey: OUTBOX,
            seq,
        };
        match hint.encode(community_id, &own.author) {
            Ok(hint) => self.send_to_mesh_impl(
                community_id,
                &CommunityEnvelope::Control(ControlPayload::HeadAdvanced { hint }),
            ),
            Err(error) => tracing::debug!(community_id, %error, "own outbox hint not encoded"),
        }
    }
}
