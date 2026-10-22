//! A community's per-event attendee lists (plan C7.15, on the Host at
//! E3.1 E4). RSVPs are member-authored governance entries
//! (`GovernanceEntry::EventRsvp`), aggregated per event as Matrix
//! aggregates `m.calendar.rsvp` relations; `EventRsvpChanged` gossip is the
//! fast path between merges.

use std::collections::HashMap;

use rekindle_governance::state::GovernanceState;
use rekindle_types::event::{EventRsvp, RsvpStatus};

use super::community_events::event_key;
use crate::host::community::state::CommunityState;

/// Rebuild `event_rsvps_by_event` from merged governance: every live
/// event's RSVPs, with our own answers from `my_event_rsvps` laid over
/// them (ours are authoritative locally, and a write that has not merged
/// yet is not in governance). Each list is ordered by pseudonym.
pub(super) fn project(cs: &mut CommunityState, gov: &GovernanceState) {
    let mut by_event: HashMap<String, Vec<EventRsvp>> = HashMap::new();
    for (event_id, rsvps) in gov.live_event_rsvps() {
        let list = by_event.entry(event_key(&event_id)).or_default();
        for (member, status) in rsvps {
            upsert(list, &hex::encode(member.0), status.as_wire_str());
        }
    }
    if let Some(me) = cs.my_pseudonym_key.clone() {
        let live: Vec<String> = gov.events.keys().map(event_key).collect();
        for (event_id, status) in &cs.my_event_rsvps {
            if live.contains(event_id) {
                upsert(by_event.entry(event_id.clone()).or_default(), &me, status);
            }
        }
    }
    cs.event_rsvps_by_event = by_event;
}

/// Apply one member's answer, normalized to the three wire values.
pub(super) fn apply(cs: &mut CommunityState, event_id: &str, pseudonym_key: &str, status: &str) {
    let status = RsvpStatus::from_request(status).as_wire_str();
    upsert(
        cs.event_rsvps_by_event
            .entry(event_id.to_owned())
            .or_default(),
        pseudonym_key,
        status,
    );
}

fn upsert(list: &mut Vec<EventRsvp>, pseudonym_key: &str, status: &str) {
    if let Some(existing) = list.iter_mut().find(|e| e.pseudonym_key == pseudonym_key) {
        status.clone_into(&mut existing.status);
    } else {
        list.push(EventRsvp {
            pseudonym_key: pseudonym_key.to_owned(),
            status: status.to_owned(),
        });
        list.sort_by(|a, b| a.pseudonym_key.cmp(&b.pseudonym_key));
    }
}
