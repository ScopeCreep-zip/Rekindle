//! Poll lifecycle, signing, and the channel reads.

use std::sync::Arc;

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Sign our own presence row with this community's pseudonym.
    ///
    /// W26: the slot seed is shared with every member, so any member can
    /// write any slot. This signature is the only thing that makes the
    /// row ours.
    pub(super) fn sign_presence_impl(
        &self,
        community_id: &str,
        signing_bytes: &[u8],
    ) -> Option<Vec<u8>> {
        let secret = self
            .ctx
            .signing_key
            .read()
            .as_ref()
            .map(|k| *k.as_bytes())?;
        // Both from `rekindle-crypto`: the pseudonym derivation and the
        // signature have to be the same pair every reader verifies with,
        // and that crate is the one both shells share.
        let pseudonym =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
        Some(
            rekindle_crypto::group::pseudonym::sign_with_pseudonym(&pseudonym, signing_bytes)
                .to_vec(),
        )
    }

    /// Run one presence poll tick for a community.
    /// Builds a fresh adapter for the tick rather than taking
    /// `self: &Arc<Self>`, because the trait method is `&self` and the
    /// orchestrator needs an owned `Arc<D>`. The adapter is one `Arc`
    /// clone of the context, so this is cheap — the same shape the
    /// desktop adapter uses.
    pub(super) async fn run_poll_tick_impl(&self, community_id: &str) -> Result<(), String> {
        let adapter = Arc::new(Self::new(Arc::clone(&self.ctx)));
        rekindle_presence::community::presence_poll_tick(adapter, community_id).await
    }
}
