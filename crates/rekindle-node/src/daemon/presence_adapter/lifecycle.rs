//! Poll lifecycle, signing, and the channel reads.

use std::sync::Arc;

use rekindle_presence::deps::PresenceError;
use rekindle_protocol::dht::community::channel_record::ChannelMessage;

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

    /// Read every message in one channel record.
    ///
    /// The same `rekindle-protocol` call the desktop makes — this is a
    /// DHT read, not a local-store read, so the daemon having no message
    /// database is irrelevant to it.
    ///
    /// It expects an **SMPL channel segment record**: one record per
    /// `(channel, segment)` with a subkey per member. That is now what
    /// both tracks write. While the daemon still allocated a DFLT
    /// `DhtLog` per member per channel this read returned nothing for
    /// any key minted here, which is why it looked like a stub.
    pub(super) async fn read_channel_record_impl(
        &self,
        record_key: &str,
        member_count: u32,
    ) -> Result<Vec<ChannelMessage>, PresenceError> {
        let node = self.transport().ok_or(PresenceError::NotAttached)?;
        let dht = node.dht().map_err(|_| PresenceError::NotAttached)?;
        rekindle_transport::broadcast::dht::channel_smpl::read_messages(
            dht.routing_context(),
            record_key,
            member_count,
        )
        .await
        .map_err(|e| PresenceError::Dht(e.to_string()))
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

    /// Record the poll's shutdown handle so lock/logout can stop it.
    pub(super) fn install_poll_shutdown_impl(
        &self,
        community_id: &str,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
    ) {
        self.ctx
            .presence_shutdowns
            .lock()
            .insert(community_id.to_string(), shutdown_tx);
    }

    /// Stop every running presence poll.
    ///
    /// Called on lock and on shutdown: the polls hold an `Arc` to the
    /// context and write presence rows, so leaving them running after a
    /// lock would keep advertising a member who is no longer unlocked.
    pub fn stop_all_polls(ctx: &crate::daemon::dispatch::DaemonContext) {
        let handles: Vec<_> = ctx.presence_shutdowns.lock().drain().collect();
        for (community_id, tx) in handles {
            // The receiver is inside a `select!`; a closed channel ends
            // the loop just as well as a message, so a full or dropped
            // channel is not an error worth surfacing.
            let _ = tx.try_send(());
            tracing::debug!(
                community = %&community_id[..16.min(community_id.len())],
                "presence poll stopped"
            );
        }
    }
}
