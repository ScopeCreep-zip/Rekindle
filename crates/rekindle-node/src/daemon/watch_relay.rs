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
//! Only the receive half lives here: the send half relayed changes of the
//! transport's per-channel-record watches, deleted with the channel slot
//! record (plan E3.3 slice 8). A member's outbox change reaches peers as
//! the S7 ring's `HeadAdvanced` hint instead.

use rekindle_transport::InboundHandler;
use tracing::debug;

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
        let Some(transport) = self
            .transport
            .read()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        else {
            return;
        };

        let Ok(Some(value)) = rekindle_transport::broadcast::dht_writes::read_once_str(
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
                observer = %observer_pseudonym,
                "watch relay: content hash mismatch, dropping"
            );
            return;
        }

        self.on_value_change(record_key, vec![subkey], Some(value))
            .await;
    }
}
