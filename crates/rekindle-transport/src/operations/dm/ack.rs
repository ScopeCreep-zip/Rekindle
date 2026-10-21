//! Delivery receipts (plan E2.5, D22; Careless Whisper §VII): a `DmAck`
//! names entries of the peer's DM stream we committed. It goes only to an
//! accepted friend, only after the entry is durably committed and validated
//! as user content (never for typing, presence or control frames; a
//! reaction, edit or delete is not applied before E6.4, so none is
//! acknowledged), coalesced per peer after a uniform 2–10 s random delay
//! from the Host's generator, and held while we are Invisible, then flushed
//! on leaving it. A received `DmAck` is the only source of `delivered`.
//! Our read marker (`ReadUpTo`, [`super::receipts`]) rides the same
//! coalescer, delay and Invisible hold.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmAck, ReadUpTo, MAX_DM_ACK_SEQS};
use rekindle_types::friend::FriendshipState;
use rekindle_utils::RandomSource;

use super::{client_id_text, receipts, send};
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

/// The receipts waiting per peer: delivery acknowledgements and our read
/// marker, sent together by one flush.
#[derive(Debug, Default)]
pub struct PendingAcks {
    by_peer: HashMap<String, BTreeSet<u64>>,
    read_up_to: HashMap<String, u64>,
    scheduled: HashSet<String>,
}

/// What one flush to a peer sends.
#[derive(Debug, Default, PartialEq, Eq)]
struct Held {
    seqs: BTreeSet<u64>,
    read_up_to: Option<u64>,
}

impl PendingAcks {
    /// Hold `seq` for `peer`; `true` when a flush must be scheduled (none
    /// is).
    fn add(&mut self, peer: &str, seq: u64) -> bool {
        self.by_peer.entry(peer.to_owned()).or_default().insert(seq);
        self.scheduled.insert(peer.to_owned())
    }

    /// Hold our read marker `seq` for `peer` (the highest one wins);
    /// `true` when a flush must be scheduled.
    fn add_read(&mut self, peer: &str, seq: u64) -> bool {
        let marker = self.read_up_to.entry(peer.to_owned()).or_default();
        *marker = (*marker).max(seq);
        self.scheduled.insert(peer.to_owned())
    }

    /// The flush for `peer` ran: what it sends, unless it stays held.
    fn take(&mut self, peer: &str, hold: bool) -> Option<Held> {
        self.scheduled.remove(peer);
        if hold {
            return None;
        }
        let held = Held {
            seqs: self.by_peer.remove(peer).unwrap_or_default(),
            read_up_to: self.read_up_to.remove(peer),
        };
        (held != Held::default()).then_some(held)
    }

    fn held_peers(&self) -> Vec<String> {
        let peers: BTreeSet<&String> = self.by_peer.keys().chain(self.read_up_to.keys()).collect();
        peers.into_iter().cloned().collect()
    }

    /// Forget everything (lock).
    pub fn clear(&mut self) {
        self.by_peer.clear();
        self.read_up_to.clear();
        self.scheduled.clear();
    }
}

/// The client id of `body` when it is user content we acknowledge.
#[must_use]
pub fn acknowledged_id(body: &DirectBody) -> Option<String> {
    match body {
        DirectBody::DmText(text) => Some(client_id_text(&text.client_id)),
        DirectBody::StreamRecord(r) => Some(super::stream::stream_record_client_id(&r.record_key)),
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
    if stream_seq == 0 || !accepted(host, from) {
        return;
    }
    let first = host.stores().acks.lock().add(from, stream_seq);
    if first {
        schedule(host, from);
    }
}

/// Whether `peer` is an accepted friend (never a pending, blocked or
/// unknown one): the only peer a receipt goes to.
pub(crate) fn accepted(host: &dyn FriendHost, peer: &str) -> bool {
    state::friend(host, peer).is_some_and(|f| f.friendship_state == FriendshipState::Accepted)
}

/// Hold our read marker `seq` for `peer` on the coalescer.
pub(crate) fn hold_read(host: &dyn FriendHost, peer: &str, seq: u64) {
    let first = host.stores().acks.lock().add_read(peer, seq);
    if first {
        schedule(host, peer);
    }
}

/// Flush `peer` after the coalescing delay.
fn schedule(host: &dyn FriendHost, peer: &str) {
    let (owned, peer, wait) = (host.owned(), peer.to_owned(), delay(host.rng()));
    host.scope().spawn_or_drop("dm receipts", async move {
        tokio::time::sleep(wait).await;
        flush(owned.as_ref(), &peer).await;
    });
}

/// Send `peer`'s waiting receipts, unless we are Invisible.
async fn flush(host: &dyn FriendHost, peer: &str) {
    let invisible = host.invisible();
    let Some(held) = host.stores().acks.lock().take(peer, invisible) else {
        return;
    };
    let seqs: Vec<u64> = held.seqs.into_iter().collect();
    let mut bodies: Vec<DirectBody> = seqs
        .chunks(MAX_DM_ACK_SEQS)
        .map(|chunk| {
            DirectBody::DmAck(DmAck {
                stream_seqs: chunk.to_vec(),
            })
        })
        .collect();
    // The marker is checked again as it leaves: the setting or the
    // friendship may have changed while it waited.
    if let Some(stream_seq) = held.read_up_to {
        if receipts::may_send(host, peer).await {
            bodies.push(DirectBody::ReadUpTo(ReadUpTo { stream_seq }));
        }
    }
    for body in &bodies {
        // Never queued: an ack that does not leave is asked for again by
        // the peer's resend, and a marker is sent again with the next read.
        match send::send_once(host, peer, body).await {
            Ok(true) => {}
            Ok(false) => tracing::debug!(to = %peer, "no route — receipt dropped"),
            Err(e) => tracing::debug!(to = %peer, error = %e, "receipt not sent"),
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
        let seqs: Vec<u64> = acks.take("bob", false).unwrap().seqs.into_iter().collect();
        assert_eq!(seqs, [1, 3, 4]);
    }

    #[test]
    fn the_read_marker_shares_the_flush_and_is_held_while_invisible() {
        let mut acks = PendingAcks::default();
        assert!(acks.add_read("bob", 4));
        assert!(!acks.add_read("bob", 2), "one flush per peer");
        assert!(!acks.add("bob", 7), "acks ride the same flush");
        // Invisible: nothing leaves, and the marker stays held.
        assert_eq!(acks.take("bob", true), None);
        assert_eq!(acks.held_peers(), ["bob"]);
        assert!(acks.add_read("bob", 6), "a held peer schedules again");
        // Leaving Invisible sends one marker, the highest.
        let held = acks.take("bob", false).unwrap();
        assert_eq!(held.read_up_to, Some(6));
        assert_eq!(held.seqs.into_iter().collect::<Vec<_>>(), [7]);
        assert_eq!(acks.take("bob", false), None, "sent once");
        assert!(acks.held_peers().is_empty());
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
