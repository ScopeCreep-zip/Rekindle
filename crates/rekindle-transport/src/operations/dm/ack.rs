//! Delivery receipts (plan E2.5, D22; Careless Whisper §VII): a `DmAck`
//! names entries of the peer's DM stream we committed. It goes only to an
//! accepted friend, only after the entry is durably committed and validated
//! as user content (never for typing, presence or control frames; a
//! reaction, edit or delete is not applied before E6.4, so none is
//! acknowledged), coalesced per peer after a uniform 2–10 s random delay
//! from the Host's generator, and held while we are Invisible, then flushed
//! on leaving it. A received `DmAck` is the only source of `delivered`.
//! It is cumulative (register 2026-10-24, Tox `buffer_start`): `through`
//! is our committed cursor on the peer's chain, every entry below it
//! applied, so a later ack covers a lost one; the seqs it lists are the
//! entries committed since the last flush, a resend already held among
//! them (Briar answers every duplicate by acking again).
//! Our read marker (`ReadUpTo`, [`super::receipts`]) rides the same
//! coalescer, delay and Invisible hold.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmAck, ReadUpTo, MAX_DM_ACK_SEQS};
use rekindle_types::friend::FriendshipState;
use rekindle_utils::RandomSource;

use rekindle_sync::stream::DM_STREAM;

use super::{client_id_text, receipts, send};
use crate::operations::direct::frame::Opened;
use crate::operations::friend::{state, FriendHost};

/// The coalescing delay's bounds, milliseconds.
pub const ACK_DELAY_MIN_MS: u64 = rekindle_types::message::DM_ACK_DELAY_MIN_MS;
pub const ACK_DELAY_MAX_MS: u64 = rekindle_types::message::DM_ACK_DELAY_MAX_MS;

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
    /// Per peer, the newest [`APPLIED_KEPT`] entries we opened: the
    /// digest of the exact frame with its stream seq.
    applied: HashMap<String, VecDeque<([u8; 32], u64)>>,
}

/// How many opened entries per peer an exact copy is matched against: a
/// copy is the peer's re-ask of rows one of our acks named and lost, and
/// one `DmAck` names at most `MAX_DM_ACK_SEQS` seqs, so twice that covers
/// two lost flushes in a row. Memory only: after a restart a copy draws
/// no ack, and the re-ask after the row lands (framed afresh,
/// `dm::outbox::rehint`) is acknowledged as a new arrival.
const APPLIED_KEPT: usize = 2 * MAX_DM_ACK_SEQS;

/// What one flush to a peer sends.
#[derive(Debug, Default, PartialEq, Eq)]
struct Held {
    /// A `DmAck` is owed: these seqs, or none past `through` (a
    /// duplicate's re-ack).
    seqs: Option<BTreeSet<u64>>,
    read_up_to: Option<u64>,
}

impl PendingAcks {
    /// Hold `seq` for `peer`; `true` when a flush must be scheduled (none
    /// is).
    fn add(&mut self, peer: &str, seq: u64) -> bool {
        self.by_peer.entry(peer.to_owned()).or_default().insert(seq);
        self.scheduled.insert(peer.to_owned())
    }

    /// A `DmAck` carrying our cursor alone is owed to `peer` (a chain diff
    /// moved it); `true` when a flush must be scheduled.
    fn add_through(&mut self, peer: &str) -> bool {
        self.by_peer.entry(peer.to_owned()).or_default();
        self.scheduled.insert(peer.to_owned())
    }

    /// Entry `seq` of `peer`'s stream opened from the frame `digest`.
    fn remember(&mut self, peer: &str, digest: [u8; 32], seq: u64) {
        let kept = self.applied.entry(peer.to_owned()).or_default();
        if kept.len() == APPLIED_KEPT {
            kept.pop_front();
        }
        kept.push_back((digest, seq));
    }

    /// The stream seq of `peer`'s entry whose frame was exactly `digest`.
    fn applied_seq(&self, peer: &str, digest: &[u8; 32]) -> Option<u64> {
        self.applied
            .get(peer)?
            .iter()
            .find(|(d, _)| d == digest)
            .map(|(_, seq)| *seq)
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
            seqs: self.by_peer.remove(peer),
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
        self.applied.clear();
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

/// `body` from `from`, entry `stream_seq` of their stream and `chain_seq` of
/// their chain (0: none), was committed: record it received, step our
/// cursor on their chain ([`step_cursor`]) and schedule its acknowledgement.
pub(crate) async fn committed(
    host: &dyn FriendHost,
    from: &str,
    body: &DirectBody,
    stream_seq: u64,
    chain_seq: u64,
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
    step_cursor(host, from, chain_seq).await;
    let first = host.stores().acks.lock().add(from, stream_seq);
    if first {
        schedule(host, from);
    }
}

/// A chain diff of `from`'s stream applied entries or stored a gap, so our
/// cursor on them moved (live finding 76b: a diff that applied hints held
/// past a gap sent no ack, and the sender re-asked rows the cursor already
/// covered): a `DmAck` carrying the new `through` follows, after the same
/// coalescing delay, joined with any seqs already held.
pub(crate) fn cursor_moved(host: &dyn FriendHost, from: &str) {
    if !accepted(host, from) {
        return;
    }
    let first = host.stores().acks.lock().add_through(from);
    if first {
        schedule(host, from);
    }
}

/// The digest an exact copy of a frame is matched by: BLAKE3 of the
/// whole frame (header, ciphertext and the wire kind).
#[must_use]
pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    blake3::hash(bytes).into()
}

/// `opened` came from the frame `digest`: an entry of its sender's stream
/// that we acknowledge is remembered, so an exact copy of it is
/// acknowledged again ([`duplicate`]).
pub(crate) fn opened(host: &dyn FriendHost, opened: &Opened, digest: [u8; 32]) {
    if opened.stream_seq > 0 && acknowledged_id(&opened.body).is_some() {
        host.stores()
            .acks
            .lock()
            .remember(&opened.sender_hex, digest, opened.stream_seq);
    }
}

/// A copy, `digest`, of a frame claiming `from` that the ratchet or replay
/// cache already saw: their ack of it may be lost, so an exact copy of an
/// entry of theirs we opened is acknowledged again by its own stream seq
/// (register 2026-10-24, Briar `DatabaseComponentImpl.java#L961-997` acks
/// the message id), after the same 2–10 s coalescing delay. A copy that
/// matches nothing we opened (a header seen with another ciphertext, a
/// replayed page with its kind rewritten) draws no ack, and only an
/// accepted friend is answered.
pub(crate) fn duplicate(host: &dyn FriendHost, from: &str, digest: &[u8; 32]) {
    if !accepted(host, from) {
        return;
    }
    let first = {
        let mut acks = host.stores().acks.lock();
        let Some(seq) = acks.applied_seq(from, digest) else {
            return tracing::debug!(from = %from, "duplicate matches no entry we opened: no ack");
        };
        acks.add(from, seq)
    };
    if first {
        schedule(host, from);
    }
}

/// D23's receiver rule on an applied entry's chain seq
/// (`repo::stream_cursors::step`), whatever path brought it; a gap is held
/// for the chain diff the hint started. So `DmAck.through`, the cursor, is
/// the highest contiguous chain seq applied by any path.
async fn step_cursor(host: &dyn FriendHost, from: &str, chain_seq: u64) {
    if chain_seq == 0 {
        return;
    }
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let (writer, now) = (from.to_owned(), host.clock().now_ms_i64());
    let stepped = state::db_call!(host, "step the chain cursor", move |conn| {
        rekindle_db::repo::stream_cursors::step(conn, &owner, DM_STREAM, &writer, chain_seq, now)
    })
    .await;
    match stepped {
        Ok(at) if chain_seq > at + 1 => {
            tracing::debug!(from = %from, chain_seq, cursor = at, "hint past a gap: held for the diff");
        }
        Ok(_) => {}
        Err(e) => tracing::debug!(from = %from, error = %e, "chain cursor not stepped"),
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
    let mut bodies = match held.seqs {
        Some(seqs) => acks(
            &seqs.into_iter().collect::<Vec<_>>(),
            cursor(host, peer).await,
        ),
        None => Vec::new(),
    };
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

/// The `DmAck`s for `seqs` and our cursor `through`: one per
/// `MAX_DM_ACK_SEQS` seqs, or one carrying `through` alone (a duplicate's
/// re-ack); none when there is nothing to name.
fn acks(seqs: &[u64], through: u64) -> Vec<DirectBody> {
    if seqs.is_empty() {
        return (through > 0)
            .then(|| {
                DirectBody::DmAck(DmAck {
                    stream_seqs: Vec::new(),
                    through,
                })
            })
            .into_iter()
            .collect();
    }
    seqs.chunks(MAX_DM_ACK_SEQS)
        .map(|chunk| {
            DirectBody::DmAck(DmAck {
                stream_seqs: chunk.to_vec(),
                through,
            })
        })
        .collect()
}

/// Our committed cursor on `peer`'s chain (`stream_cursors`, advanced only
/// after an entry is applied): a `DmAck`'s `through`; 0 when unread.
async fn cursor(host: &dyn FriendHost, peer: &str) -> u64 {
    let Ok(owner) = state::owner(host) else {
        return 0;
    };
    let writer = peer.to_owned();
    let read = state::db_call!(host, "read the ack cursor", move |conn| {
        rekindle_db::repo::stream_cursors::get(conn, &owner, DM_STREAM, &writer)
    })
    .await;
    read.unwrap_or_else(|e| {
        tracing::debug!(to = %peer, error = %e, "ack cursor not read");
        0
    })
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
    let mut through_raised = 0;
    if ack.through > 0 {
        match outbox.acknowledge_chain_through(from, ack.through).await {
            Ok(n) => through_raised = n,
            Err(e) => tracing::warn!(from = %from, error = %e, "DmAck through not applied"),
        }
    }
    let (mut settled, mut unmatched) = (Vec::new(), Vec::new());
    for seq in &ack.stream_seqs {
        let Ok(stream_seq) = i64::try_from(*seq) else {
            unmatched.push(*seq);
            continue;
        };
        match outbox.acknowledge(from, stream_seq).await {
            Ok(true) => settled.push(*seq),
            Ok(false) => unmatched.push(*seq),
            Err(e) => tracing::warn!(from = %from, error = %e, "DmAck not applied"),
        }
    }
    tracing::debug!(from = %from, through = ack.through, through_raised,
        covered = ?ack.stream_seqs, ?settled, ?unmatched, "DmAck ingested");
}

#[cfg(test)]
mod tests {
    use rekindle_codec::capnp_codec::direct::{DmReaction, DmText, Typing};

    use super::*;

    /// A diff's ack carries `through` alone (finding 76b), and joins the
    /// seqs already held for the same flush.
    #[test]
    fn a_moved_cursor_owes_a_through_ack() {
        let mut acks = PendingAcks::default();
        assert!(acks.add_through("p"));
        let held = acks.take("p", false).expect("an ack is owed");
        assert_eq!(held.seqs, Some(BTreeSet::new()));
        assert_eq!(super::acks(&[], 3).len(), 1);
        assert!(acks.add("p", 5));
        assert!(!acks.add_through("p"));
        assert_eq!(
            acks.take("p", false).unwrap().seqs,
            Some(BTreeSet::from([5]))
        );
    }

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
        let seqs: Vec<u64> = acks
            .take("bob", false)
            .unwrap()
            .seqs
            .unwrap()
            .into_iter()
            .collect();
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
        assert_eq!(held.seqs.unwrap().into_iter().collect::<Vec<_>>(), [7]);
        assert_eq!(acks.take("bob", false), None, "sent once");
        assert!(acks.held_peers().is_empty());
    }

    #[test]
    fn only_an_exact_copy_of_an_opened_entry_is_acknowledged_by_its_seq() {
        let mut acks = PendingAcks::default();
        let (frame, forged) = (super::digest(b"frame 4"), super::digest(b"frame 4'"));
        acks.remember("bob", frame, 4);
        assert_eq!(acks.applied_seq("bob", &frame), Some(4));
        assert_eq!(acks.applied_seq("bob", &forged), None, "another ciphertext");
        assert_eq!(acks.applied_seq("eve", &frame), None, "another sender");
        for seq in 5..5 + APPLIED_KEPT as u64 {
            acks.remember("bob", super::digest(&seq.to_be_bytes()), seq);
        }
        assert_eq!(acks.applied_seq("bob", &frame), None, "bounded");
        acks.clear();
        assert_eq!(
            acks.applied_seq("bob", &super::digest(&5u64.to_be_bytes())),
            None
        );
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
