//! The desktop's per-event attendee lists (plan C7.15).
//!
//! RSVPs are member-authored governance entries
//! (`GovernanceEntry::EventRsvp`), aggregated per event as Matrix
//! aggregates `m.calendar.rsvp` relations; `EventRsvpChanged` gossip is
//! the fast path in between merges. They used to ride every member's
//! presence row, which a heartbeat slot cannot hold.

use rekindle_governance::state::GovernanceState;
use rekindle_types::event::{EventRsvp, RsvpStatus};

use crate::state::CommunityState;

/// Rebuild `event_rsvps_by_event` from merged governance: every live
/// event's RSVPs, with our own answers from `my_event_rsvps` laid over
/// them (ours are authoritative locally, and a write that has not landed
/// yet is not in governance).
pub fn rebuild_from_governance(community: &mut CommunityState, gov: &GovernanceState) {
    let mut by_event: std::collections::HashMap<String, Vec<EventRsvp>> = gov
        .live_event_rsvps()
        .into_iter()
        .map(|(event_id, rsvps)| {
            let list = rsvps
                .into_iter()
                .map(|(member, status)| EventRsvp {
                    pseudonym_key: hex::encode(member.0),
                    status: status.as_wire_str().to_string(),
                })
                .collect();
            (event_key(&event_id), list)
        })
        .collect();
    if let Some(me) = community.my_pseudonym_key.clone() {
        for (event_id, status) in &community.my_event_rsvps {
            let live =
                gov.events
                    .contains_key(&crate::services::community_event_runtime::parse_event_id(
                        event_id,
                    ));
            if live {
                upsert(by_event.entry(event_id.clone()).or_default(), &me, status);
            }
        }
    }
    community.event_rsvps_by_event = by_event;
}

/// Apply one member's answer: from our own command, or from an
/// `EventRsvpChanged` envelope. The status is normalized to the three
/// wire values.
pub fn apply(community: &mut CommunityState, event_id: &str, pseudonym_key: &str, status: &str) {
    let status = RsvpStatus::from_request(status).as_wire_str();
    upsert(
        community
            .event_rsvps_by_event
            .entry(event_id.to_string())
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
            pseudonym_key: pseudonym_key.to_string(),
            status: status.to_string(),
        });
        list.sort_by(|a, b| a.pseudonym_key.cmp(&b.pseudonym_key));
    }
}

/// The `evt_<hex>` id the desktop keys events by.
fn event_key(event_id: &rekindle_types::id::EventId) -> String {
    format!("evt_{}", hex::encode(event_id.0))
}
