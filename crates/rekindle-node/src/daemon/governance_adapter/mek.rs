//! Keys the governance runtime hands the daemon (hydration, JoinAccepted).
//!
//! Installed into `rekindle_transport::crypto::mek::MekCache` through
//! `MekCacheAdapter`, so the shared convergence rule (no downgrade, lowest
//! rank at an equal generation) applies, and stored in the vault, from
//! which unlock reloads them. Reads go through `mek_rotation::key_provider`.

use rekindle_governance_runtime::deps::MekSnapshot;
use rekindle_transport::crypto::mek::Mek;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;

use super::DaemonGovernanceAdapter;

/// A channel id as the governance runtime passes it (32 hex), as a scope.
fn channel_scope(channel_id: &str) -> Option<KeyScope> {
    ChannelId::from_hex(channel_id).map(KeyScope::Channel)
}

impl DaemonGovernanceAdapter<'_> {
    /// Cache the key and, when the cache takes it (the convergence rule may
    /// refuse a stale or outranked one), store it in the vault, so the next
    /// unlock reloads it.
    fn install(&self, community_id: &str, scope: KeyScope, mek: &MekSnapshot) {
        let key = Mek::from_bytes(mek.key_bytes, mek.generation);
        let taken = rekindle_mek_rotation::ChannelMekCache::insert(
            &crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
                &self.ctx.mek_cache,
            )),
            community_id,
            scope,
            key.clone(),
        );
        if !taken {
            return;
        }
        let guard = self.ctx.vault.lock();
        let Some(vault) = guard.as_ref() else {
            return;
        };
        if let Err(e) = rekindle_vault::typed::mek::persist_mek(vault, community_id, scope, &key) {
            tracing::error!(community = %community_id, %scope, error = %e, "MEK not stored");
        }
    }

    pub(super) fn insert_community_mek_impl(&self, community_id: &str, mek: &MekSnapshot) {
        self.install(community_id, KeyScope::Community, mek);
    }

    pub(super) fn insert_channel_mek_impl(
        &self,
        community_id: &str,
        channel_id: &str,
        mek: &MekSnapshot,
    ) {
        let Some(scope) = channel_scope(channel_id) else {
            tracing::warn!(community = %community_id, channel_id, "not a channel id — key not cached");
            return;
        };
        self.install(community_id, scope, mek);
    }
}
