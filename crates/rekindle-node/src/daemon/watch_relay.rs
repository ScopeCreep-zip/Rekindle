//! Mutual Aid §14.3 — the watch relay.
//!
//! Veilid reserves `member_watch_limit` (8) signed watch slots plus
//! `public_watch_limit` (32) anonymous ones **per record**, so in a
//! community past roughly forty members most hold no watch on any given
//! record. That is a hard limit in `veilid-core`, not something the
//! application can configure away.
//!
//! The architecture's answer is cooperative: members who hold a slot
//! relay what they see to members who do not. It is Plumtree's
//! eager/lazy push (Leitão et al., *Epidemic Broadcast Trees*, SRDS'07)
//! with the roles assigned by Veilid rather than by us — `WatchRelay` is
//! the `IHAVE`, and the receiver's `get_dht_value` plus hash check is
//! the `GRAFT`. Principle 12: "a member who relays for peers gets
//! relayed for in return."
//!
//! Split from `handler.rs` because it is one coherent concern — the
//! send half hangs off `on_value_change`, the receive half off
//! `on_community_gossip`, and they only make sense together.

use rekindle_transport::InboundHandler;
use tracing::{debug, trace};

use super::handler::DaemonHandler;

impl DaemonHandler {
    /// Mutual Aid §14.3 — a peer holding a watch slot is telling us a
    /// record's subkey changed.
    ///
    /// This is Plumtree's lazy push. Veilid reserves only
    /// `member_watch_limit` (8) signed watch slots plus
    /// `public_watch_limit` (32) anonymous ones **per record**, so in a
    /// community of any size most members hold no watch on any given
    /// record. Peers that do hold one relay the notification — the
    /// `IHAVE` — and we pull the value ourselves.
    ///
    /// The relay carries a `content_hash` and no ciphertext, on purpose:
    /// gossip is unencrypted at the envelope layer, so shipping the
    /// value would leak it to every hop. The hash lets us verify that
    /// what we fetched is what the observer saw.
    pub(super) async fn on_watch_relay(
        &self,
        record_key: &str,
        subkey: u32,
        content_hash: &str,
        observer_pseudonym: &str,
    ) {
        let Some(transport) = self.transport.read().clone() else {
            return;
        };

        // If we hold our own watch on this record, our value-change
        // callback covers the same change — skip the redundant fetch.
        // Members without a slot fall through, which is the whole point
        // of the relay.
        let we_watch = self
            .subscriptions
            .read()
            .as_ref()
            .is_some_and(|manager| manager.has_watch(record_key));
        if we_watch {
            trace!(
                record_key,
                subkey,
                "watch relay: own watch covers this record"
            );
            return;
        }

        let Ok(Some(value)) = rekindle_transport::broadcast::dht_writes::get(
            transport.as_ref(),
            record_key,
            subkey,
            true,
        )
        .await
        else {
            return;
        };

        let actual = blake3::hash(&value).to_hex().to_string();
        if actual != content_hash {
            debug!(
                record_key,
                subkey,
                observer = &observer_pseudonym[..12.min(observer_pseudonym.len())],
                "watch relay: content hash mismatch, dropping"
            );
            return;
        }

        self.on_value_change(record_key, vec![subkey], Some(value))
            .await;
    }
}

impl DaemonHandler {
    /// Broadcast a `WatchRelay` for a change our own watch reported.
    ///
    /// The relay names the record, the subkey and a BLAKE3 hash of the
    /// new value — never the value. Gossip is unencrypted at the
    /// envelope layer, so shipping the bytes would hand a channel
    /// message's ciphertext to every forwarding hop; the hash is what a
    /// watchless peer verifies its own fetch against.
    ///
    /// Silent when we hold no watch on the record: `on_value_change`
    /// also fires for values we pulled after someone else's relay, and
    /// re-relaying those would put one change into an endless loop
    /// around the mesh (the dedup cache would break the loop, but only
    /// after the traffic had gone out).
    pub(super) fn relay_watch_change(
        &self,
        record_key: &str,
        changed_subkeys: &[u32],
        value: Option<&[u8]>,
    ) {
        let Some(value) = value else {
            // No first value means either a multi-subkey change (the
            // caller must fetch each one anyway) or a dead watch. In
            // both cases we have no hash to publish.
            return;
        };
        let Some(subkey) = changed_subkeys.first().copied() else {
            return;
        };
        let we_watch = self
            .subscriptions
            .read()
            .as_ref()
            .is_some_and(|manager| manager.has_watch(record_key));
        if !we_watch {
            return;
        }

        // Which community does this record belong to, and who are we in
        // it? A relay has to be attributable — `observer_pseudonym` is
        // what lets a receiver weigh it.
        let Some((community_id, observer)) = self.community_for_record(record_key) else {
            return;
        };

        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::WatchRelay {
            record_key: record_key.to_string(),
            subkey,
            content_hash: blake3::hash(value).to_hex().to_string(),
            observer_pseudonym: observer,
        };
        crate::daemon::gossip::send(&self.gossip_tx, &community_id, &envelope);
    }

    /// `(community_id, my_pseudonym)` for whichever community owns this
    /// record — governance, registry, or one of its channel records.
    fn community_for_record(&self, record_key: &str) -> Option<(String, String)> {
        let guard = self.session.read();
        let session = guard.as_ref()?;
        session.communities.values().find_map(|m| {
            let ours = m.governance_key == record_key
                || m.registry_key == record_key
                || m.channel_record_keys.values().any(|k| k == record_key);
            (ours && !m.pseudonym_key.is_empty())
                .then(|| (m.governance_key.clone(), m.pseudonym_key.clone()))
        })
    }
}
