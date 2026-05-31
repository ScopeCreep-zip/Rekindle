//! Pure per-event RSVP aggregation for the registry-scan
//! post-processing.
//!
//! Composes three RSVP sources into one `event_id → Vec<EventRsvpEntry>`
//! map ready for `community.event_rsvps_by_event`:
//!
//! 1. Discovered peers' presence-row `event_rsvps` (one row per peer).
//! 2. Local user's own RSVPs (read from `community.my_event_rsvps`).
//! 3. The `known_event_ids` list bounds the aggregation —
//!    presence-row RSVPs for events the local user hasn't loaded
//!    yet are dropped, so a stale snapshot doesn't surface them.
//!
//! Local user's RSVPs override any prior entry for the same
//! pseudonym (we know our own RSVPs authoritatively).

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::community::util::presence_event_id_bytes;
use crate::community::DiscoveredRow;

/// One peer's RSVP for one event. Mirrors src-tauri's
/// `EventRsvpEntry` shape so the adapter can convert into the
/// AppState type with a thin map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRsvpEntry {
    pub pseudonym_key: String,
    pub status: String,
}

/// Aggregate per-event RSVPs across discovered presence rows + the
/// local user's overrides.
///
/// `known_event_ids` is the set of event UUIDs the local user has
/// loaded (architecture §15 — events are discovered through
/// governance and persisted before any RSVP traffic is meaningful).
/// Presence-row RSVPs referencing unknown events are filtered out.
///
/// The returned vec for each event is sorted by `pseudonym_key`
/// ascending — deterministic ordering so SolidJS list keys stay
/// stable across ticks.
#[must_use]
pub fn aggregate_event_rsvps<S>(
    discovered: &[DiscoveredRow],
    my_event_rsvps: &HashMap<String, String, S>,
    known_event_ids: &[String],
    my_pseudonym: &str,
) -> HashMap<String, Vec<EventRsvpEntry>>
where
    S: BuildHasher,
{
    let event_by_presence_id: HashMap<[u8; 16], String> = known_event_ids
        .iter()
        .map(|event_id| (presence_event_id_bytes(event_id), event_id.clone()))
        .collect();

    let mut aggregated: HashMap<String, Vec<EventRsvpEntry>> = HashMap::new();

    // Discovered peers' RSVPs first (each row contributes one entry
    // per (event_id, peer) pair).
    for (_segment_index, _subkey, presence) in discovered {
        let pseudonym_key = hex::encode(presence.pseudonym_key.0);
        for rsvp in &presence.event_rsvps {
            let Some(event_id) = event_by_presence_id.get(&rsvp.event_id.0) else {
                continue;
            };
            aggregated
                .entry(event_id.clone())
                .or_default()
                .push(EventRsvpEntry {
                    pseudonym_key: pseudonym_key.clone(),
                    status: rsvp.status.clone(),
                });
        }
    }

    // Local user's RSVPs override anything peer-discovered for the
    // same pseudonym (we know our own RSVPs authoritatively).
    for (event_id, status) in my_event_rsvps {
        let entry = aggregated.entry(event_id.clone()).or_default();
        entry.retain(|e| e.pseudonym_key != my_pseudonym);
        entry.push(EventRsvpEntry {
            pseudonym_key: my_pseudonym.to_string(),
            status: status.clone(),
        });
    }

    // Deterministic ordering — keeps SolidJS list keys stable
    // across ticks even when discovery order is non-deterministic.
    for rsvps in aggregated.values_mut() {
        rsvps.sort_by(|a, b| a.pseudonym_key.cmp(&b.pseudonym_key));
    }

    aggregated
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::{EventId, PseudonymKey};
    use rekindle_types::presence::{EventRSVP, MemberPresence};

    fn row(pk: u8, event_id: &str, status: &str) -> DiscoveredRow {
        let mut bytes = [0u8; 32];
        bytes[0] = pk;
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey(bytes),
            event_rsvps: vec![EventRSVP {
                event_id: EventId(presence_event_id_bytes(event_id)),
                status: status.to_string(),
            }],
            ..Default::default()
        };
        (0u32, 0u32, presence)
    }

    #[test]
    fn empty_inputs_yield_empty_map() {
        let agg = aggregate_event_rsvps::<std::collections::hash_map::RandomState>(
            &[],
            &HashMap::new(),
            &[],
            "",
        );
        assert!(agg.is_empty());
    }

    #[test]
    fn discovered_rsvp_for_known_event_is_aggregated() {
        let agg = aggregate_event_rsvps::<std::collections::hash_map::RandomState>(
            &[row(1, "event-42", "yes")],
            &HashMap::new(),
            &["event-42".to_string()],
            "",
        );
        let rsvps = agg.get("event-42").expect("entry");
        assert_eq!(rsvps.len(), 1);
        assert_eq!(rsvps[0].status, "yes");
    }

    #[test]
    fn discovered_rsvp_for_unknown_event_is_dropped() {
        let agg = aggregate_event_rsvps::<std::collections::hash_map::RandomState>(
            &[row(1, "stale-event", "yes")],
            &HashMap::new(),
            &["other-event".to_string()],
            "",
        );
        assert!(agg.is_empty());
    }

    #[test]
    fn my_rsvp_overrides_discovered_for_same_pseudonym() {
        // The `row` helper builds pseudonym bytes as `[first, 0, 0, …]`,
        // so the matching `my_pk` hex must use the same shape.
        let mut my_pk_bytes = [0u8; 32];
        my_pk_bytes[0] = 7;
        let my_pk = hex::encode(my_pk_bytes);
        let mut my_rsvps = HashMap::new();
        my_rsvps.insert("event-42".to_string(), "maybe".to_string());
        let agg = aggregate_event_rsvps(
            &[row(7, "event-42", "no")],
            &my_rsvps,
            &["event-42".to_string()],
            &my_pk,
        );
        let rsvps = agg.get("event-42").expect("entry");
        assert_eq!(rsvps.len(), 1);
        assert_eq!(rsvps[0].pseudonym_key, my_pk);
        assert_eq!(rsvps[0].status, "maybe");
    }

    #[test]
    fn output_is_sorted_by_pseudonym_ascending() {
        let agg = aggregate_event_rsvps::<std::collections::hash_map::RandomState>(
            &[
                row(0xff, "event-42", "no"),
                row(0x01, "event-42", "yes"),
                row(0x80, "event-42", "maybe"),
            ],
            &HashMap::new(),
            &["event-42".to_string()],
            "",
        );
        let rsvps = agg.get("event-42").expect("entry");
        let keys: Vec<&str> = rsvps.iter().map(|e| e.pseudonym_key.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn my_rsvp_adds_new_event_entry_when_no_peer_rsvped() {
        let my_pk = hex::encode([1u8; 32]);
        let mut my_rsvps = HashMap::new();
        my_rsvps.insert("event-99".to_string(), "yes".to_string());
        let agg = aggregate_event_rsvps(
            &[],
            &my_rsvps,
            &["event-99".to_string()],
            &my_pk,
        );
        let rsvps = agg.get("event-99").expect("entry");
        assert_eq!(rsvps.len(), 1);
        assert_eq!(rsvps[0].pseudonym_key, my_pk);
    }
}
