//! Reclaiming registry slots whose occupant is no longer a member.
//!
//! ## Why this has to exist
//!
//! `communities-governance.md` says leaving frees a slot — *"zero own
//! registry slot… The slot becomes available for reuse"* — but nothing
//! implemented it, and the obvious implementation cannot work. Veilid
//! has no per-subkey delete (`delete_dht_record` is documented
//! local-only: it "does not delete it from the network"), and
//! `set_dht_value` has no unset. Worse, the claim path asks
//! `inspect_dht_record_present_subkeys`, which reports *ever written*
//! rather than *currently occupied* — so a departed member's slot reads
//! as taken forever no matter what is written into it.
//!
//! Left alone that turns `SLOTS_PER_SEGMENT` into a **lifetime-joins**
//! cap rather than a concurrent-members one, behind a hard
//! `MAX_SEGMENTS = 8` wall, and every leaked slot is permanent recurring
//! work: `scan_segment_raw` fetches and W26-verifies every non-empty row
//! on every presence tick, for every member, across every segment.
//!
//! ## Why it looks like this
//!
//! The bound is Veilid's, not ours. `DHTSchema` is accepted only by
//! `create_dht_record` — there is no `set_schema`, and `DHTSchemaSMPL`
//! exposes `members()` read-only — so the member list is immutable at
//! creation. That is precisely why all 255 slot keypairs derive from one
//! shared seed: it is the only way to seat members whose keys are not
//! yet known. It also means any member can write any slot, which
//! `communities-overview.md` already states as principle 7 ("Any member
//! can write any entry; readers independently check authority").
//!
//! So reclamation must be reader-derived, and it follows MLS
//! (RFC 9420 §7.1), whose ratchet tree is bounded the same way ours is:
//! a Remove *blanks* a leaf, and an Add "MUST use the leftmost leaf in
//! the tree that is either blank or contains a LeafNode with a
//! `leaf_node_source` of `key_package`. If there are no such leaves, the
//! sender MUST extend the tree to the right by one leaf." Blank-first,
//! extend-last — which here means reclaim the lowest reusable slot
//! before triggering a Plate Gate expansion.
//!
//! Jami is deliberately *not* the model here. Its swarms store
//! membership as Git commits in an unbounded repository and never
//! reclaim anything ("History can not be deleted"; banned certificates
//! stay in `/banned` forever), so it has no answer to a fixed-capacity
//! structure — it simply never has one.
//!
//! ## What counts as reclaimable
//!
//! MLS blanks a leaf through an authenticated Commit, so a removed
//! member's slot is freed *without their cooperation*. That matters: a
//! banned member will never tombstone themselves. The CRDT gives us the
//! same thing for free — a slot whose occupant governance has banned is
//! reclaimable with no new wire format at all.
//!
//! **Staleness is deliberately not a reason.** A member offline for a
//! month still owns their slot; MLS blanks on explicit Remove, never on
//! inactivity, and evicting on a missed heartbeat would make a laptop
//! lid a membership event.
//!
//! Voluntary departure is the one case still uncovered: there is no
//! `MemberDeparted` entry and `GovernanceState` has no departed set, so
//! a member who leaves cleanly currently leaves no trace anywhere. That
//! needs a *signed* marker rather than zeroed bytes — an empty payload
//! carries no signature, so treating it as a tombstone would let any
//! member free any other member's slot and collide two members onto it.
//! Tracked separately; this module reclaims what is already provable.

use rekindle_records::port::OpenIntent;
use std::collections::HashSet;

use rekindle_governance::state::GovernanceState;
use rekindle_presence::{parse_and_classify_row, ClassifiedRow};

use crate::deps::GovernanceRuntimeDeps;

/// Slots that are occupied on the wire but hold no valid member.
///
/// Only called when a segment would otherwise be declared full, so the
/// common join pays one `inspect` and nothing more. The cost here is one
/// fetch per occupied slot, which is the same read the joiner would have
/// done during a Plate Gate expansion anyway — and expansion is the
/// alternative this is trying to avoid.
///
/// Returned lowest-index-first so the caller reuses the leftmost
/// reclaimable slot, per the MLS rule. That also keeps segments dense,
/// which matters because presence polling costs scale with the number of
/// segments, not the number of live members.
pub(super) async fn reclaimable_slots<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    occupied: &[u32],
    gov_state: &GovernanceState,
) -> Vec<u32> {
    let banned: HashSet<String> = gov_state.bans.iter().map(|p| hex::encode(p.0)).collect();

    let mut out = Vec::new();
    // An unborrowable registry reclaims nothing: no slot is evidence of
    // vacancy.
    let Ok(lease) = deps
        .acquire_record(registry_key, None, OpenIntent::Check)
        .await
    else {
        return out;
    };
    for &subkey in occupied {
        // A read failure is not evidence of vacancy — a slot we could
        // not fetch stays occupied. Reclaiming on a transient DHT error
        // would hand a live member's slot to a joiner.
        let Ok(Some(raw)) = deps.get_dht_value(lease, subkey, false).await else {
            continue;
        };
        if is_reclaimable(&raw, &banned) {
            out.push(subkey);
        }
    }
    deps.release_record(lease).await;
    out.sort_unstable();
    out
}

/// Whether one registry row leaves its slot free for a new member.
///
/// Split out from the fetch loop so the rule is testable without a DHT:
/// this is the whole of the reclamation policy.
fn is_reclaimable(raw: &[u8], banned: &HashSet<String>) -> bool {
    // The staleness arguments are inert here by construction:
    // `parse_and_classify_row` only uses them to decide whether an
    // accepted row is *online*, never whether it is accepted. Passing
    // zeroes makes that explicit rather than smuggling an eviction
    // policy in through a threshold.
    // Two reasons, one outcome (clippy::match_same_arms forbids splitting
    // them, so the distinction lives here):
    //
    //   * No member here, or nobody who can *prove* they are one. A row
    //     failing W26 was written by someone who does not hold the
    //     pseudonym it claims — under a shared slot seed that signature
    //     is the only authorship proof there is, so it establishes no
    //     claim on the slot.
    //   * Banned: governance says this pseudonym is out. The MLS
    //     blank-on-Remove case, and why bans needed no new wire format —
    //     the CRDT already carries the removal.
    match parse_and_classify_row(raw, banned, 0, 0) {
        ClassifiedRow::EmptyPayload
        | ClassifiedRow::MalformedJson
        | ClassifiedRow::InvalidSignatureLength
        | ClassifiedRow::SignatureRejected
        | ClassifiedRow::Banned
        | ClassifiedRow::Departed => true,
        // A valid member, however long they have been away.
        ClassifiedRow::Accepted(_) => false,
    }
}

#[cfg(test)]
#[path = "reclaim_tests.rs"]
mod tests;
