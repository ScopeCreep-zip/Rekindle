//! The fake's receipts over a range of its stream (`read_through`,
//! `deliver`).

use rekindle_types::message::SendState;

use super::Fake;
use crate::state::{merge, PeerAck, StateInput};
use crate::OutboxRow;

impl Fake {
    /// The peer's `ack` of entries `lo..=hi` of `stream`.
    pub(super) fn ack_range(
        &self,
        stream: &str,
        (lo, hi): (i64, i64),
        ack: PeerAck,
    ) -> Vec<(i64, String)> {
        let mut s = self.store.lock();
        let covered = |r: &OutboxRow| r.stream == stream && (lo..=hi).contains(&r.stream_seq);
        // A covered row not landed is marked read and kept.
        let unlanded: Vec<(i64, String)> = s
            .rows
            .iter()
            .filter(|r| covered(r) && r.state == SendState::Queued)
            .filter(|r| !matches!(s.acks.get(&r.client_id), Some(a) if *a == ack || *a == PeerAck::Read))
            .map(|r| (r.stream_seq, r.client_id.clone()))
            .collect();
        for (_, id) in &unlanded {
            s.acks.insert(id.clone(), ack);
        }
        let mut raised: Vec<(i64, String)> = s
            .rows
            .iter()
            .filter(|r| covered(r) && r.state != SendState::Queued)
            .map(|r| (r.stream_seq, r.client_id.clone()))
            .collect();
        raised.extend(unlanded);
        s.rows
            .retain(|r| !(covered(r) && r.state != SendState::Queued));
        for (st, seq, id, state) in &mut s.settled {
            let to = merge(*state, StateInput::Peer(ack));
            if st == stream && (lo..=hi).contains(seq) && to != *state {
                *state = to;
                raised.push((*seq, id.clone()));
            }
        }
        raised
    }
}
