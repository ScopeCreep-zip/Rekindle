//! History for joiners (plan D16; ADR 0012 "History for joiners", the
//! Matrix MSC4268 pattern): the inviter delivers the bundle of community
//! key generations the joiner's role may read, and the joiner installs
//! every generation it receives, older ones included.
//!
//! - **Who may read history:** a member whose effective community
//!   permissions hold `READ_HISTORY` (`rekindle-types` `permissions.rs`;
//!   the plan derives a generation's visibility from that permission, with
//!   no separate field). A member without it is owed only the current
//!   generation: the generations before its join are excluded.
//! - **Which generations:** every one from 1 to the current that we hold,
//!   in memory or in the vault, oldest first; a generation we never held
//!   (we joined after it without history) is not ours to give.
//! - **Install of an older generation:** the convergence rule refuses a
//!   downgrade of the key we send under, so an older generation is kept at
//!   its own generation (the cache retains every generation, the vault
//!   indexes each) without moving the current one. The outbox entries a
//!   catch-up passed for want of it (`channel/outbox_hold.rs`: an entry
//!   older than our first key moves the cursor on) are read again: the
//!   community's outbox cursors are forgotten and every outbox caught up,
//!   an entry already applied coming back as a duplicate the ingest drops.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::MekDistributeDeps;
use rekindle_transport::crypto::mek::Mek;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::permissions::READ_HISTORY;

use super::moderation::pseudonym;
use super::HostGovernance;

/// The community generations a joiner is owed when the key we send under
/// is at `current`, oldest first: every one when its role may read
/// history, else only the current.
#[must_use]
pub(crate) fn generations_owed(current: u64, reads_history: bool) -> Vec<u64> {
    if reads_history {
        (1..=current).collect()
    } else {
        vec![current]
    }
}

impl HostGovernance {
    /// Whether `member_hex`'s effective community permissions, at the
    /// Host's clock, hold `READ_HISTORY`.
    pub(crate) fn member_reads_history(&self, community_id: &str, member_hex: &str) -> bool {
        let Some(member) = pseudonym(member_hex) else {
            return false;
        };
        self.ctx
            .communities
            .governance_state(community_id)
            .is_some_and(|state| {
                rekindle_governance::permissions::compute_permissions(
                    &member,
                    None,
                    &state,
                    self.ctx.clock.now_secs(),
                ) & READ_HISTORY
                    != 0
            })
    }

    /// The community keys `member_hex` is owed with `current`, oldest
    /// first ([`generations_owed`]), each one we hold at exactly its
    /// generation.
    pub(crate) async fn key_bundle<D: MekDistributeDeps + Sync + ?Sized>(
        &self,
        deps: &D,
        community_id: &str,
        member_hex: &str,
        current: MediaEncryptionKey,
    ) -> Vec<MediaEncryptionKey> {
        let reads_history = self.member_reads_history(community_id, member_hex);
        let mut bundle = Vec::new();
        for generation in generations_owed(current.generation(), reads_history) {
            if generation == current.generation() {
                bundle.push(current.clone());
                continue;
            }
            if let Some(mek) = Self::held_generation(deps, community_id, generation).await {
                bundle.push(mek);
            } else {
                tracing::debug!(
                    community = community_id,
                    generation,
                    "an older community key we never held is not in the joiner's bundle"
                );
            }
        }
        bundle
    }

    /// The community key at exactly `generation`, from memory or the vault.
    async fn held_generation<D: MekDistributeDeps + Sync + ?Sized>(
        deps: &D,
        community_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        if let Some(mek) = deps
            .cache()
            .get(community_id, KeyScope::Community, generation)
        {
            return Some(mek);
        }
        deps.persist()
            .load_mek_for_generation(community_id, KeyScope::Community, generation)
            .await
            .ok()
            .flatten()
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .map(|bytes| MediaEncryptionKey::from_bytes(bytes, generation))
    }

    /// Keep `mek`, older than the key we send under, at its own generation:
    /// cached (never replacing a key held at it), stored in the vault, and
    /// the community's outboxes read again from the start. Returns whether
    /// it was new.
    pub(crate) fn install_history_key(
        &self,
        community_id: &str,
        scope: KeyScope,
        mek: &Mek,
    ) -> bool {
        let generation = mek.generation();
        {
            let mut cache = self.ctx.mek_cache.write();
            if cache
                .get_generation(community_id, scope, generation)
                .is_some()
            {
                return false;
            }
            cache.insert(community_id, scope, mek.clone());
        }
        {
            let guard = self.ctx.vault.lock();
            if let Some(vault) = guard.as_ref() {
                if let Err(e) =
                    rekindle_vault::typed::mek::persist_mek(vault, community_id, scope, mek)
                {
                    tracing::error!(community = %community_id, %scope, error = %e, "history key not stored");
                }
            }
        }
        tracing::info!(community_id, %scope, generation, "history key installed (D16)");
        self.spawn_outbox_reread(community_id);
        true
    }
}

#[cfg(test)]
#[path = "key_history_tests.rs"]
mod tests;
