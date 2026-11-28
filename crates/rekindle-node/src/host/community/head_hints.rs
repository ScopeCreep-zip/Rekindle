//! What the watch ring and its `HeadAdvanced` hints have read of each
//! member record (plan E3.2 S7, `evidence/slot-integrity.md` §4.3/§4.5):
//! the seqs read from the DHT, the frontier catch-ups in flight, and the
//! records whose op-log head moved since their frontier was last relayed.
//! In memory only (`CommunityState::head_hints` is `#[serde(skip)]`): the
//! login sweep reads every frontier again after a restart.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_governance_runtime::anti_entropy::{FrontierRead, SeenFrontiers};
use rekindle_types::dht_layout::member_record::{FRONTIER, GOV_HEAD};
use rekindle_utils::log_scrub::tag;

/// The seqs `HeadAdvanced` hints were acted on at, per (member record,
/// subkey): a seq is recorded only once read from the DHT, never from the
/// hint (`evidence/slot-integrity.md` §4.3, "hints are self-verifying";
/// register S7l), so one member's `u64::MAX` hint mutes nothing.
#[derive(Debug, Clone, Default)]
pub struct HeadHints {
    read: BTreeMap<(String, u32), u64>,
    /// The frontier (`M`/3) seqs read, shared with the anti-entropy
    /// sweeps (review S7m part 2 P2): a frontier the ring or a hint read
    /// is not read again by a sweep, nor the reverse. Its clones share it.
    frontiers: SeenFrontiers,
    /// One frontier catch-up per record at a time (review S7m 83f7fc6b
    /// P1): the claim is taken under the communities write lock before
    /// the DHT reads, so a ring change and the hints of up to four
    /// relayers for one frontier write cost one catch-up.
    in_flight: BTreeMap<String, Flight>,
    /// Ring records whose op-log head (`M`/2) moved in a change that did
    /// not also name the frontier: the frontier write that follows is the
    /// author's append, the one frontier change a ring watcher relays.
    head_moved: BTreeSet<String>,
    /// The op-log head (`M`/2) seq each ring record was last seen at: a
    /// change that names the head moves it only past this seq. Veilid
    /// reports one value change from each node holding our watch, so a
    /// change can come again for a value already handled (netsim
    /// `ring_frontier`, LF7c: `[2, 3]`, then `[2]` and `[3]` for the same
    /// seqs relayed the frontier twice).
    heads: BTreeMap<String, u64>,
}

/// A frontier catch-up and the highest sequence any later trigger named.
#[derive(Debug, Clone)]
struct Flight {
    claimed: Option<u64>,
    missed: Option<u64>,
}

impl HeadHints {
    /// Is a hint at `hinted` worth a read: above the seq last read, or
    /// for a subkey never read? Records nothing. A subkey's first value
    /// is at seq 0 (Veilid `ValueSeqNum::next` from NONE,
    /// `veilid_api/types/dht/value_seq_num.rs:24`), so an unread subkey
    /// is not "read at 0": a fresh member record's first outbox entry
    /// is hinted at seq 0 and must be read.
    #[must_use]
    pub fn worth_reading(&self, record: &str, subkey: u32, hinted: u64) -> bool {
        self.read_at(record, subkey)
            .is_none_or(|read| hinted > read)
    }

    /// Record the seq the DHT read returned: `true` when it moved past
    /// the last one read (any seq for a subkey never read), so the hint
    /// is acted on.
    pub fn record_read(&mut self, record: &str, subkey: u32, read: u64) -> bool {
        if subkey == FRONTIER {
            return self.frontiers.mark_read(record, read);
        }
        let seen = self.read.entry((record.to_owned(), subkey));
        match seen {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(read);
                true
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let moved = read > *slot.get();
                *slot.get_mut() = (*slot.get()).max(read);
                moved
            }
        }
    }

    /// Claim `record`'s frontier catch-up for a trigger naming `seq` (a
    /// hint's seq, or the ring change's inspected seq): `false` while
    /// another catch-up of it runs (its seq is then kept as missed, see
    /// [`Self::finish_frontier`]) or, unless `forced` (a ring change, never
    /// gated on an inspect a lagging replica answers), when `seq` was read.
    pub fn claim_frontier(&mut self, record: &str, seq: Option<u64>, forced: bool) -> bool {
        let last_read = self.read_at(record, FRONTIER);
        let unread = last_read.map_or(0, |read| read.saturating_add(1));
        if let Some(flight) = self.in_flight.get_mut(record) {
            flight.missed = flight.missed.max(Some(seq.unwrap_or(unread)));
            self.frontiers
                .set_minimum(record, flight.claimed.max(flight.missed));
            tracing::debug!(
                record_tag = %tag(record),
                ?seq,
                forced,
                claimed = ?flight.claimed,
                missed = ?flight.missed,
                "frontier trigger joined a read in flight"
            );
            return false;
        }
        if !forced && seq.is_some_and(|seq| !self.worth_reading(record, FRONTIER, seq)) {
            tracing::debug!(record_tag = %tag(record), ?seq, ?last_read, "frontier hint refused: sequence already read");
            return false;
        }
        self.frontiers.set_minimum(record, seq);
        self.in_flight.insert(
            record.to_owned(),
            Flight {
                claimed: seq,
                missed: None,
            },
        );
        tracing::debug!(record_tag = %tag(record), ?seq, ?last_read, forced, "frontier trigger claimed");
        true
    }

    /// Record a complete read that reached the highest trigger. A newer
    /// trigger that joined the flight gets its own forced read. Only a
    /// second attempt for a claim goes through D8's refetch cap.
    pub fn finish_frontier(
        &mut self,
        record: &str,
        read: Option<u64>,
        refetched: bool,
    ) -> Option<FrontierRead> {
        let Some(flight) = self.in_flight.remove(record) else {
            tracing::debug!(record_tag = %tag(record), ?read, "frontier finish refused: no claim");
            return None;
        };
        let minimum = flight.claimed.max(flight.missed);
        if let Some(read) = read.filter(|read| Some(*read) >= minimum) {
            self.record_read(record, FRONTIER, read);
        }
        let recorded = self.read_at(record, FRONTIER);
        let (next, reason) = if minimum.is_none() && recorded.is_none() {
            (None, "trigger had no sequence and no complete read")
        } else if minimum <= recorded {
            (None, "read reached the claim")
        } else if flight.missed > flight.claimed {
            (
                Some(FrontierRead::Hinted { minimum }),
                "newer joined trigger needs its forced read",
            )
        } else if let Some(minimum) = minimum.filter(|_| !refetched) {
            (
                Some(FrontierRead::Refetch { minimum }),
                "claim needs a capped refetch",
            )
        } else {
            (None, "capped refetch finished below the claim")
        };
        if next.is_some() {
            self.in_flight.insert(
                record.to_owned(),
                Flight {
                    claimed: minimum,
                    missed: None,
                },
            );
        } else {
            self.frontiers.set_minimum(record, None);
        }
        tracing::debug!(
            record_tag = %tag(record),
            ?read,
            ?recorded,
            ?next,
            claimed = ?flight.claimed,
            missed = ?flight.missed,
            refetched,
            reason,
            "frontier read finished"
        );
        next
    }

    /// Does the ring change on `subkeys` of `record` relay a frontier
    /// hint? Only when the same write advanced the op-log head (review
    /// S7m 83f7fc6b P2): the change names both, or the head moved in an
    /// earlier change (the writer appends `M`/2, then writes `M`/3). A
    /// republish after a rebuild moves the frontier alone and stays a
    /// ring-local signal, so ops spread hop by hop, not by mesh floods.
    /// The head moved only when `head_seq`, its seq as the change was
    /// inspected, is past the one last seen: a change reported again for
    /// a head already seen moves nothing (LF7c).
    pub fn frontier_follows_head(
        &mut self,
        record: &str,
        subkeys: &[u32],
        head_seq: Option<u64>,
    ) -> bool {
        let head = subkeys.contains(&GOV_HEAD) && self.head_advanced(record, head_seq);
        if !subkeys.contains(&FRONTIER) {
            if head {
                self.head_moved.insert(record.to_owned());
            }
            return false;
        }
        self.head_moved.remove(record) || head
    }

    /// Forget every record not in `records` (the current members' records,
    /// after a rebuild): a departed member's entries are never read again.
    pub fn retain_records(&mut self, records: &BTreeSet<String>) {
        self.frontiers.retain_records(records);
        self.read.retain(|(record, _), _| records.contains(record));
        self.head_moved.retain(|record| records.contains(record));
        self.heads.retain(|record, _| records.contains(record));
    }

    /// The community's frontier reads, shared with its sweeps.
    #[must_use]
    pub fn frontiers(&self) -> SeenFrontiers {
        self.frontiers.clone()
    }

    /// Whether `seq` moves `record`'s head past the one last seen (any seq
    /// for a record never seen), which it then is. An unknown seq (the
    /// inspect failed) proves no advance.
    fn head_advanced(&mut self, record: &str, seq: Option<u64>) -> bool {
        let Some(seq) = seq else {
            return false;
        };
        if self.heads.get(record).is_some_and(|seen| seq <= *seen) {
            return false;
        }
        self.heads.insert(record.to_owned(), seq);
        true
    }

    fn read_at(&self, record: &str, subkey: u32) -> Option<u64> {
        if subkey == FRONTIER {
            return self.frontiers.get(record);
        }
        self.read.get(&(record.to_owned(), subkey)).copied()
    }
}

#[cfg(test)]
#[path = "head_hints_tests.rs"]
mod tests;
