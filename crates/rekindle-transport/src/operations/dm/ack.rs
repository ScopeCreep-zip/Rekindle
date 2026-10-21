//! Delivery receipts (plan E2.5, D22; Careless Whisper §VII): a `DmAck`
//! names entries of the peer's DM stream we committed. It goes only to an
//! accepted friend, only after the entry is durably committed and validated
//! as user content (never for typing, presence or control frames; a
//! reaction, edit or delete is not applied before E6.4, so none is
//! acknowledged), coalesced per peer after a uniform 2–10 s random delay
//! from the Host's generator, and held while we are Invisible, then flushed
//! on leaving it. A received `DmAck` is the only source of `delivered`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmAck, MAX_DM_ACK_SEQS};
use rekindle_types::friend::FriendshipState;
use rekindle_utils::RandomSource;

use super::{client_id_text, send};
use crate::operations::friend::{state, FriendHost};

/// The coalescing delay's bounds, milliseconds.
pub const ACK_DELAY_MIN_MS: u64 = 2_000;
pub const ACK_DELAY_MAX_MS: u64 = 10_000;

/// The delay for a uniform `draw` in `[0, MAX - MIN]`.
fn delay_from(draw: u64) -> Duration {
    Duration::from_millis(ACK_DELAY_MIN_MS + draw.min(ACK_DELAY_MAX_MS - ACK_DELAY_MIN_MS))
}

/// A uniform delay in `[ACK_DELAY_MIN_MS, ACK_DELAY_MAX_MS]`.
#[must_use]
pub fn delay(rng: &dyn RandomSource) -> Duration {
    delay_from(rng.below(ACK_DELAY_MAX_MS - ACK_DELAY_MIN_MS + 1))
}

/// The acknowledgements waiting per peer.
#[derive(Debug, Default)]
pub struct PendingAcks {
    by_peer: HashMap<String, BTreeSet<u64>>,
    scheduled: HashSet<String>,
}

impl PendingAcks {
    /// Hold `seq` for `peer`; `true` when a flush must be scheduled (none
    /// is).
    fn add(&mut self, peer: &str, seq: u64) -> bool {
        self.by_peer.entry(peer.to_owned()).or_default().insert(seq);
        self.scheduled.insert(peer.to_owned())
    }

    /// The flush for `peer` ran: its entries, unless they stay held.
    fn take(&mut self, peer: &str, hold: bool) -> Option<BTreeSet<u64>> {
        self.scheduled.remove(peer);
        if hold {
            return None;
        }
        self.by_peer.remove(peer)
    }

    fn held_peers(&self) -> Vec<String> {
        self.by_peer.keys().cloned().collect()
    }

    /// Forget everything (lock).
    pub fn clear(&mut self) {
        self.by_peer.clear();
        self.scheduled.clear();
    }
}

/// The client id of `body` when it is user content we acknowledge.
#[must_use]
pub fn acknowledged_id(body: &DirectBody) -> Option<String> {
    match body {
        DirectBody::DmText(text) => Some(client_id_text(&text.client_id)),
        _ => None,
    }
}

/// `body` from `from`, entry `stream_seq` of their stream, was committed:
/// record it received and schedule its acknowledgement.
pub(crate) async fn committed(
    host: &dyn FriendHost,
    from: &str,
    body: &DirectBody,
    stream_seq: u64,
) {
    let Some(client_id) = acknowledged_id(body) else {
        return;
    };
    match host.outbox() {
        Ok(outbox) => {
            if let Err(e) = outbox.receive(from, &client_id).await {
                tracing::warn!(from = %from, error = %e, "received entry not recorded");
            }
        }
        Err(e) => tracing::debug!(error = %e, "no outbox: entry not recorded"),
    }
    let accepted =
        state::friend(host, from).is_some_and(|f| f.friendship_state == FriendshipState::Accepted);
    if stream_seq == 0 || !accepted {
        return;
    }
    if !host.stores().acks.lock().add(from, stream_seq) {
        return;
    }
    let (owned, peer, wait) = (host.owned(), from.to_owned(), delay(host.rng()));
    host.scope().spawn_or_drop("dm ack", async move {
        tokio::time::sleep(wait).await;
        flush(owned.as_ref(), &peer).await;
    });
}

/// Send `peer`'s waiting acknowledgements, unless we are Invisible.
async fn flush(host: &dyn FriendHost, peer: &str) {
    let invisible = host.invisible();
    let Some(seqs) = host.stores().acks.lock().take(peer, invisible) else {
        return;
    };
    let seqs: Vec<u64> = seqs.into_iter().collect();
    for chunk in seqs.chunks(MAX_DM_ACK_SEQS) {
        let ack = DirectBody::DmAck(DmAck {
            stream_seqs: chunk.to_vec(),
        });
        // Never queued: an ack that does not leave is asked for again by
        // the peer's resend.
        match send::send_once(host, peer, &ack).await {
            Ok(true) => {}
            Ok(false) => tracing::debug!(to = %peer, "no route — DmAck dropped"),
            Err(e) => tracing::debug!(to = %peer, error = %e, "DmAck not sent"),
        }
    }
}

/// We left Invisible: send every acknowledgement held meanwhile.
pub async fn flush_held(host: &dyn FriendHost) {
    let peers = host.stores().acks.lock().held_peers();
    for peer in peers {
        flush(host, &peer).await;
    }
}

/// `from` acknowledged entries of our stream to them.
pub(crate) async fn on_ack(host: &dyn FriendHost, from: &str, ack: &DmAck) {
    let outbox = match host.outbox() {
        Ok(outbox) => outbox,
        Err(e) => {
            tracing::debug!(error = %e, "DmAck with no outbox");
            return;
        }
    };
    for seq in &ack.stream_seqs {
        let Ok(seq) = i64::try_from(*seq) else {
            continue;
        };
        if let Err(e) = outbox.acknowledge(from, seq).await {
            tracing::warn!(from = %from, error = %e, "DmAck not applied");
        }
    }
}

#[cfg(test)]
mod tests {
    use rekindle_codec::capnp_codec::direct::{DmReaction, DmText, Typing};

    use super::*;

    #[test]
    fn the_delay_stays_within_two_to_ten_seconds() {
        assert_eq!(delay_from(0), Duration::from_secs(2));
        assert_eq!(delay_from(u64::MAX), Duration::from_secs(10));
        let rng = rekindle_utils::OsRandom;
        for _ in 0..1000 {
            let d = delay(&rng);
            assert!((Duration::from_secs(2)..=Duration::from_secs(10)).contains(&d));
        }
    }

    #[test]
    fn acks_coalesce_per_peer_and_stay_held_while_invisible() {
        let mut acks = PendingAcks::default();
        assert!(acks.add("bob", 3));
        assert!(!acks.add("bob", 1), "one flush per peer");
        assert!(acks.add("carol", 1));
        assert_eq!(acks.take("bob", true), None);
        assert_eq!(acks.held_peers().len(), 2);
        assert!(acks.add("bob", 4), "a held peer schedules again");
        let seqs: Vec<u64> = acks.take("bob", false).unwrap().into_iter().collect();
        assert_eq!(seqs, [1, 3, 4]);
    }

    #[test]
    fn only_text_is_acknowledged() {
        let text = DirectBody::DmText(DmText {
            client_id: [1; 16],
            body: "hi".into(),
            reply_to: None,
            expire_seconds: 0,
        });
        assert!(acknowledged_id(&text).is_some());
        assert_eq!(
            acknowledged_id(&DirectBody::Typing(Typing { typing: true })),
            None
        );
        assert_eq!(acknowledged_id(&DirectBody::Unfriended), None);
        let reaction = DirectBody::DmReaction(DmReaction {
            target_client_id: [9; 16],
            emoji: "x".into(),
            add: true,
        });
        assert_eq!(acknowledged_id(&reaction), None);
    }
}
