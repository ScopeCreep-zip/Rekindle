//! The desktop's live community and channel keys, and the
//! `ChannelKeyProvider` over them (plan D6, step B5).
//!
//! `AppState.meks` is read and written only here. Consumers read keys
//! through [`key_provider`], which answers the exact `(scope, generation)`
//! asked for from this map or the vault's per-generation history — never
//! another scope's key, never a guessed generation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_mek_rotation::{ChannelKinds, ChannelMekCache, MekHistory, MekKeyProvider};
use rekindle_types::channel_keys::{ChannelKeyProvider, KeyScope};
use rekindle_types::id::ChannelId;

use crate::state::AppState;

/// A scope's live key, when it became current, and the key it replaced.
///
/// The replaced key stays in memory so media under it still opens during
/// `MEDIA_PREVIOUS_EPOCH_GRACE` without a vault read per frame; the grace
/// itself is enforced by `rekindle_types::channel_keys::media_key`.
#[derive(Clone)]
pub struct LiveMek {
    current: MediaEncryptionKey,
    installed_at: Instant,
    previous: Option<MediaEncryptionKey>,
}

/// Install `mek` as `scope`'s live key under the convergence rule every
/// host shares: an older generation is refused (rollback), a newer one
/// replaces and parks the old key as `previous`, and an equal generation
/// is kept by the lowest election rank so every peer converges on the
/// same bytes whatever the arrival order. Returns whether it was
/// installed. The only writer of `AppState.meks`.
pub fn install_mek(
    state: &Arc<AppState>,
    community_id: &str,
    scope: KeyScope,
    mek: MediaEncryptionKey,
) -> bool {
    let mut meks = state.meks.lock();
    let key = (community_id.to_string(), scope);
    let previous = match meks.get(&key) {
        None => None,
        Some(live) if mek.generation() < live.current.generation() => {
            tracing::debug!(
                community = %community_id,
                %scope,
                incoming = mek.generation(),
                cached = live.current.generation(),
                "MEK older than the live key — refused"
            );
            return false;
        }
        Some(live) if mek.generation() == live.current.generation() => {
            if !rekindle_mek_rotation::convergence::incoming_wins_same_generation(
                live.current.election_rank().as_ref(),
                mek.election_rank().as_ref(),
            ) {
                return false;
            }
            // A same-generation winner replaces the loser outright: the
            // generation names one key, the canonical one.
            live.previous.clone()
        }
        Some(live) => Some(live.current.clone()),
    };
    meks.insert(
        key,
        LiveMek {
            current: mek,
            installed_at: Instant::now(),
            previous,
        },
    );
    true
}

/// `scope`'s live key.
pub fn current_mek(
    state: &Arc<AppState>,
    community_id: &str,
    scope: KeyScope,
) -> Option<MediaEncryptionKey> {
    state
        .meks
        .lock()
        .get(&(community_id.to_string(), scope))
        .map(|live| live.current.clone())
}

/// Forget every live key of `community_id` and return every scope whose
/// stored keys must be erased with it: the community scope, each channel
/// that held a live key, and each channel the community lists.
pub fn forget_community_keys(state: &Arc<AppState>, community_id: &str) -> Vec<KeyScope> {
    let mut scopes = vec![KeyScope::Community];
    state.meks.lock().retain(|(community, scope), _| {
        if community != community_id {
            return true;
        }
        if !scopes.contains(scope) {
            scopes.push(*scope);
        }
        false
    });
    if let Some(community) = state.communities.read().get(community_id) {
        for channel in &community.channels {
            if let Some(id) = ChannelId::from_hex(&channel.id) {
                let scope = KeyScope::Channel(id);
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
    }
    scopes
}

/// Drop every live key (logout).
pub fn clear_meks(state: &AppState) {
    state.meks.lock().clear();
}

/// `ChannelMekCache` over `AppState.meks`, for the rotation crate.
pub struct LiveMekCache {
    state: Arc<AppState>,
}

impl LiveMekCache {
    #[must_use]
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl ChannelMekCache for LiveMekCache {
    fn current(&self, community_id: &str, scope: KeyScope) -> Option<MediaEncryptionKey> {
        current_mek(&self.state, community_id, scope)
    }

    fn get(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let meks = self.state.meks.lock();
        let live = meks.get(&(community_id.to_string(), scope))?;
        std::iter::once(&live.current)
            .chain(live.previous.as_ref())
            .find(|mek| mek.generation() == generation)
            .cloned()
    }

    fn current_age(&self, community_id: &str, scope: KeyScope) -> Option<Duration> {
        self.state
            .meks
            .lock()
            .get(&(community_id.to_string(), scope))
            .map(|live| live.installed_at.elapsed())
    }

    fn insert(&self, community_id: &str, scope: KeyScope, mek: MediaEncryptionKey) -> bool {
        install_mek(&self.state, community_id, scope, mek)
    }
}

/// Persisted generations, from the vault.
struct VaultMekHistory {
    state: Arc<AppState>,
}

impl MekHistory for VaultMekHistory {
    fn load(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        let guard = self.state.keystore.lock();
        crate::keystore::load_mek_generation(guard.as_ref()?, community_id, scope, generation)
    }
}

/// Channel types, from the community's channel list.
struct CommunityChannelKinds {
    state: Arc<AppState>,
}

impl ChannelKinds for CommunityChannelKinds {
    fn is_stage(&self, community_id: &str, channel: ChannelId) -> bool {
        super::channel_is_stage(&self.state, community_id, &channel.to_hex())
    }
}

/// The scope a channel's text is under, or `None` for a non-channel id.
pub fn text_scope(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> Option<KeyScope> {
    let channel = ChannelId::from_hex(channel_id)?;
    Some(key_provider(state).scope_for_text(community_id, channel))
}

/// The scope a channel's voice and video are under, or `None` for a
/// non-channel id.
pub fn media_scope(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Option<KeyScope> {
    let channel = ChannelId::from_hex(channel_id)?;
    Some(key_provider(state).scope_for_media(community_id, channel))
}

/// Whether we hold a key for `channel_id`'s media scope.
pub fn media_key_present(state: &Arc<AppState>, community_id: &str, channel_id: &str) -> bool {
    media_scope(state, community_id, channel_id)
        .is_some_and(|scope| current_mek(state, community_id, scope).is_some())
}

/// The key provider every desktop key consumer reads through.
pub fn key_provider(state: &Arc<AppState>) -> Arc<dyn ChannelKeyProvider> {
    Arc::new(MekKeyProvider::new(
        Arc::new(LiveMekCache::new(Arc::clone(state))),
        Arc::new(VaultMekHistory {
            state: Arc::clone(state),
        }),
        Arc::new(CommunityChannelKinds {
            state: Arc::clone(state),
        }),
    ))
}
