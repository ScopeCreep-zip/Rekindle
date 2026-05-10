//! ChatService background tasks + lock.

use std::sync::atomic::Ordering;

use rekindle_types::subscription_events::{SubscriptionEvent, TypingEvent, TypingContext};

use crate::ChatError;

use super::ChatService;

impl ChatService {
    pub fn sweep_expired_skipped_keys(&self) -> Result<u64, ChatError> {
        Ok(self.vault.sweep_expired_skipped_keys()?)
    }

    pub fn evict_sessions(&self, max: u64) -> Result<Vec<[u8; 32]>, ChatError> {
        Ok(self.vault.evict_oldest_sessions(max)?)
    }

    pub fn evict_expired_dedup(&self) {
        self.pipeline.evict_expired_dedup();
    }

    /// Collect expired typing indicators and emit TypingStopped events.
    pub fn collect_expired_typers(&self) {
        let (channel_expired, dm_expired) = {
            let mut state = self.pipeline.state().write();
            (
                state.typing.collect_expired_channel_typers(),
                state.typing.collect_expired_dm_typers(),
            )
        };

        for (community, channel, who) in channel_expired {
            self.pipeline.process(SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Channel { community, channel },
                who,
            }));
        }

        for peer_key in dm_expired {
            self.pipeline.process(SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Dm { peer_key: peer_key.clone() },
                who: peer_key,
            }));
        }
    }

    /// Mark session_meta as modified. The 5s background flush will persist it.
    pub fn mark_session_dirty(&self) {
        self.session_dirty.store(true, Ordering::Release);
    }

    /// Persist session.json if the dirty flag is set.
    pub fn flush_session_meta_if_dirty(&self) -> Result<(), ChatError> {
        if !self.session_dirty.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let meta = self.session_meta.read().clone();
        let json = serde_json::to_vec_pretty(&meta)
            .map_err(|e| ChatError::Serialization(format!("session.json: {e}")))?;
        rekindle_storage::session_meta::save(&self.session_path, &self.session_mac_key, &json)
            .map_err(ChatError::Storage)?;
        tracing::debug!("session.json flushed");
        Ok(())
    }

    /// Lock the chat service. Zeroize all in-memory secrets.
    ///
    /// Order: flush session.json → cancel watches → clear caches → clear signing key.
    pub async fn lock(&self) {
        if let Err(e) = self.flush_session_meta_if_dirty() {
            tracing::warn!(error = %e, "session.json flush failed during lock — \
                changes since last flush may be lost on next startup");
        }

        for (record_key, token) in self.watches.all_tokens() {
            if let Err(e) = self.io.cancel_watch(token).await {
                tracing::debug!(record_key, error = %e, "watch cancel failed during lock");
            }
        }

        self.session_cache.clear().await;
        self.mek_cache.clear();
        self.io.clear_signing_key();

        tracing::info!("chat service locked — signing key, sessions, MEKs zeroized");
    }
}
