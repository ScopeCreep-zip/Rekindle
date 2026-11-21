//! Keys the governance runtime hands the daemon (hydration).
//!
//! Installed into `rekindle_transport::crypto::mek::MekCache` through
//! `MekCacheAdapter`, so the shared convergence rule (no downgrade, lowest
//! rank at an equal generation) applies, and stored in the vault, from
//! which unlock reloads them. Reads go through `mek_rotation::key_provider`.

use rekindle_governance_runtime::deps::MekSnapshot;
use rekindle_transport::crypto::mek::Mek;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::ChannelId;

use rekindle_governance_runtime::GovernanceRuntimeDeps as _;

use super::HostGovernance;

/// A channel id as the governance runtime passes it (32 hex), as a scope.
fn channel_scope(channel_id: &str) -> Option<KeyScope> {
    ChannelId::from_hex(channel_id).map(KeyScope::Channel)
}

impl HostGovernance {
    /// The unlock's MEK stage (plan E3.1 C2c), on both modes, after the
    /// warm-cache stage filled the store: the vault's latest key of every
    /// joined community and of each of its channels, into the cache through
    /// the convergence rule (a restore racing a live rotation cannot clobber
    /// it), and the shell's cache told of each one taken
    /// (`ShellHooks::on_mek_installed`, until E3.5). A key the vault lacks
    /// stays absent until a peer's `MekTransfer` brings it
    /// (`recover_behind_meks` asks only for a held key behind its
    /// generation); minting one here would fork the community. Then every
    /// joined community's queued key transfers behind the held key are
    /// requeued at it, as an install requeues them: a restored key bypasses
    /// [`Self::install`], so an app stop between a rotation's install and
    /// its requeue would otherwise leave the older rows superseded with no
    /// replacement (S7m-1b2, D22, ADR 0018).
    pub(crate) async fn restore_stored_meks(&self) {
        let mut scopes: Vec<(String, KeyScope)> = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            .flat_map(|community| {
                std::iter::once((community.id.clone(), KeyScope::Community)).chain(
                    community
                        .channels
                        .iter()
                        .filter_map(|channel| channel_scope(&channel.id))
                        .map(|scope| (community.id.clone(), scope)),
                )
            })
            .collect();
        // The store is a HashMap: a defined order for the shell's hooks.
        scopes.sort_by_cached_key(|(community, scope)| (community.clone(), scope.to_string()));
        let mut communities: Vec<String> = scopes.iter().map(|(c, _)| c.clone()).collect();
        communities.dedup();
        let stored: Vec<(String, KeyScope, Mek)> = {
            let guard = self.ctx.vault.lock();
            let Some(vault) = guard.as_ref() else {
                return;
            };
            scopes
                .into_iter()
                .filter_map(|(community, scope)| {
                    let mek = rekindle_vault::typed::mek::load_latest_mek(vault, &community, scope);
                    if mek.is_none() && scope == KeyScope::Community {
                        tracing::warn!(%community, "community key not in the vault — waiting for a peer's MekTransfer");
                    }
                    Some((community, scope, mek?))
                })
                .collect()
        };
        let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
            &self.ctx.mek_cache,
        ));
        let mut restored = 0usize;
        let mut keyed: Vec<String> = Vec::new();
        for (community, scope, mek) in stored {
            let (key_bytes, generation) = (*mek.as_bytes(), mek.generation());
            if !rekindle_mek_rotation::ChannelMekCache::insert(&cache, &community, scope, mek) {
                continue;
            }
            restored += 1;
            if let Some(shell) = &self.ctx.shell {
                shell.on_mek_installed(&community, scope, key_bytes, generation);
            }
            if !keyed.contains(&community) {
                keyed.push(community);
            }
        }
        tracing::info!(restored, "MEKs reloaded from the vault");
        // A post kept across the lock whose key was installed before its
        // release ran (plan E3.2 S7l).
        for community in keyed {
            crate::host::adapters::channel::release_waiting_posts(self, &community);
        }
        for community in communities {
            self.requeue_key_transfers(&community).await;
        }
    }

    /// Cache the key and, when the cache takes it (the convergence rule may
    /// refuse a stale or outranked one), store it in the vault, so the next
    /// unlock reloads it. Returns whether the cache took it.
    fn install(&self, community_id: &str, scope: KeyScope, mek: &MekSnapshot) -> bool {
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
            return false;
        }
        if let Some(shell) = &self.ctx.shell {
            shell.on_mek_installed(community_id, scope, mek.key_bytes, mek.generation);
        }
        {
            let guard = self.ctx.vault.lock();
            let Some(vault) = guard.as_ref() else {
                return true;
            };
            if let Err(e) =
                rekindle_vault::typed::mek::persist_mek(vault, community_id, scope, &key)
            {
                tracing::error!(community = %community_id, %scope, error = %e, "MEK not stored");
            }
        }
        // The posts composed before this key are released to their writes
        // (plan E3.2 S7l, F9-1).
        crate::host::adapters::channel::release_waiting_posts(self, community_id);
        // The outbox entries a joiner held back for want of a key are read
        // now (`channel/outbox_read.rs`).
        self.spawn_outbox_catchup(community_id);
        // A joiner still owed an older generation is owed this one now
        // (review S7m part 1 F2).
        if scope == KeyScope::Community {
            let (host, community) = (self.clone(), community_id.to_owned());
            self.scope()
                .spawn_or_drop("key transfers requeued", async move {
                    host.requeue_key_transfers(&community).await;
                });
        }
        true
    }

    /// A key a rotator or an inviter delivered (`MekTransfer`, D16),
    /// installed as every other key is: cached under the convergence rule,
    /// told to the shell, stored in the vault, and every outbox caught up
    /// again. Both hosts' `MekTransfer` handlers install through here
    /// (finding CB3: the desktop's own install started no catch-up).
    /// Returns whether the cache took it.
    pub fn install_transferred_mek(&self, community_id: &str, scope: KeyScope, mek: &Mek) -> bool {
        let generation = mek.generation();
        let snapshot = MekSnapshot {
            generation,
            key_bytes: *mek.as_bytes(),
        };
        let taken = self.install(community_id, scope, &snapshot);
        if taken {
            tracing::info!(community_id, %scope, generation, "transferred key installed");
        }
        taken
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
