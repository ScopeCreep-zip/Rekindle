//! A fresh device's catch-up of the communities its identity is already a
//! member of (plan E3.2 S7d-keys K5, ADR 0011, ADR 0022 rule 4), the first
//! part of the unlock's community stage (`login_stage.rs`).
//!
//! An imported identity's database starts empty (`identity/transfer.rs`,
//! `install_imported`); its communities are named only by the exported
//! identity vault, whose `communities` namespace holds `K_M(p)`, our member
//! record's owner, for each one. Each such community with no local row is
//! walked from its anchor alone (`anchor_walk::walk_from_anchor`), our
//! pseudonym recomputed from the identity secret and the anchor
//! (`derive_join_identity`); a community whose merged state does not hold
//! us (left, kicked or banned) is not restored. Its row takes what the
//! walked state and the vault hold, the walked ops are stored and merged,
//! and this device's first governance write is `enrol_device`: its own
//! device key and `M(p,d)`, never another device's (rule 4). Nothing is
//! written to the community before the merge completes.

use rekindle_governance_runtime::anchor_walk::walk_from_anchor;
use rekindle_governance_runtime::deps::{GovernanceOpStore as _, GovernanceRuntimeDeps as _};
use rekindle_governance_runtime::join::derive_join_identity;
use rekindle_vault::VaultKey;

use super::HostGovernance;

impl HostGovernance {
    /// The communities the identity vault holds `K_M(p)` for and we hold
    /// no row of, in id order.
    fn vault_communities_not_held(&self) -> Vec<String> {
        let listed = {
            let guard = self.ctx.vault.lock();
            let Some(vault) = guard.as_ref() else {
                return Vec::new();
            };
            match vault.list(
                VaultKey::MemberRecordOwner {
                    community: String::new(),
                }
                .namespace(),
            ) {
                Ok(keys) => keys,
                Err(error) => {
                    tracing::warn!(%error, "the vault's communities could not be listed");
                    return Vec::new();
                }
            }
        };
        let held = self.ctx.communities.map();
        let held = held.read();
        listed
            .into_iter()
            .filter_map(|key| match key {
                VaultKey::MemberRecordOwner { community } => Some(community),
                _ => None,
            })
            .filter(|community| !held.contains_key(community))
            .collect()
    }

    /// Restore every vault-listed community we hold no row of (module doc).
    pub(super) async fn restore_vault_communities(&self) {
        for anchor in self.vault_communities_not_held() {
            match self.restore_community(&anchor).await {
                Ok(true) => {
                    tracing::info!(community = %anchor, "community restored on this device");
                }
                Ok(false) => {
                    tracing::info!(community = %anchor, "not a member in the walked state; not restored");
                }
                Err(error) => {
                    tracing::warn!(community = %anchor, %error, "community restore failed");
                }
            }
        }
    }

    /// Walk `anchor`, record our membership from the walked state, store and
    /// merge its ops, then enrol this device. False when the walked state
    /// does not hold us.
    async fn restore_community(&self, anchor: &str) -> Result<bool, String> {
        let secret = self
            .identity_secret()
            .ok_or("the identity secret is unavailable")?;
        let identity = derive_join_identity(&secret, anchor);
        let walked = walk_from_anchor(self, anchor)
            .await
            .map_err(|e| format!("walk from the anchor: {e}"))?;
        let state = &walked.merged.state;
        let Some(member) = state.members.get(&identity.pseudo) else {
            return Ok(false);
        };
        let community_name =
            (state.metadata.as_ref()).map_or_else(|| walked.cert.name.clone(), |m| m.name.clone());
        let slot_seed = {
            let guard = self.ctx.vault.lock();
            let vault = guard.as_ref().ok_or("no identity is unlocked")?;
            rekindle_vault::typed::community_keys::load_slot_seed(vault, anchor)
        }
        .and_then(|h| hex::decode(h).ok())
        .and_then(|b| <[u8; 32]>::try_from(b).ok());
        let membership = rekindle_transport::session::CommunityMembership {
            anchor: anchor.to_owned(),
            pseudonym_key: identity.pseudo_hex.clone(),
            // The member registry slot (pre-E3.4) is claimed in the SMPL
            // registry, never named by an op: the walked state holds no
            // registry key or slot of ours (ADR 0011 superseded the slot
            // model), so the row records none; `segment_index` `None` is
            // the field's documented "unknown".
            registry_key: String::new(),
            slot_index: 0,
            segment_index: None,
            community_name,
            slot_seed,
            // No key held yet: 0 is the "no community key" sentinel, as at
            // a join (`join.rs`); the keys arrive through MEK recovery.
            mek_generation: 0,
        };
        crate::host::session::record_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            &self.ctx.communities,
            crate::host::session::NewMembership {
                membership,
                registry_keypair: None,
                // `K_M(p)` and the card's HPKE secret are already in the
                // exported identity vault.
                member_record_owner: None,
                member_record: Some(member.member_record.clone()),
                member_hpke_secret: None,
                genesis_cert: Some(walked.cert_bytes.clone()),
                joined_at: self.ctx.clock.now_ms_i64(),
            },
        )?;
        for op in &walked.held {
            self.store_governance_op(anchor, op)
                .await
                .map_err(|e| format!("op not stored: {e}"))?;
        }
        rekindle_governance_runtime::rebuild_governance(self, anchor).await;
        self.enrol_device(anchor)
            .await
            .map_err(|e| format!("device enrolment: {e}"))?;
        Ok(true)
    }
}
