//! Roster assembly and the `VoiceRoster` handler: building the peer list
//! a present member broadcasts to a joiner, and absorbing an inbound
//! roster into the local transport.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::VoiceRosterEntry;

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};
use crate::transport::VoiceTransport;

use super::join::send_confirmed_if_first;

/// Build the roster a present member broadcasts to a joiner. Routes
/// come straight from the transport (the route each peer advertised in
/// its VoiceJoin — authoritative for voice), and SELF is appended with
/// `our_route_blob()` so the joiner learns about the broadcaster too —
/// without self, a member that joined first would never appear in a
/// later joiner's roster.
pub(super) async fn roster_entries(
    deps: &dyn VoiceSignalingDeps,
    my_pk: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
) -> Vec<VoiceRosterEntry> {
    let peers = transport.lock().await.peer_named_entries();
    build_roster_entries(my_pk, deps.our_route_blob(), deps.my_display_name(), peers)
}

/// Pure roster assembly: map the transport's `(pseudonym, route, name)`
/// peers to entries, then append SELF (deduped) with `my_route` +
/// `my_name`. Self must be present or a member that joined first never
/// appears in a later joiner's roster. Extracted from `roster_entries`
/// so the self-inclusion invariant is unit-testable without a deps mock.
fn build_roster_entries(
    my_pk: &str,
    my_route: Vec<u8>,
    my_name: Option<String>,
    peers: Vec<(String, Vec<u8>, Option<String>)>,
) -> Vec<VoiceRosterEntry> {
    let mut entries: Vec<VoiceRosterEntry> = peers
        .into_iter()
        .map(
            |(pseudonym_key, route_blob, display_name)| VoiceRosterEntry {
                pseudonym_key,
                route_blob,
                muted: false,
                deafened: false,
                display_name,
            },
        )
        .collect();

    if !my_pk.is_empty() && !entries.iter().any(|e| e.pseudonym_key == my_pk) {
        entries.push(VoiceRosterEntry {
            pseudonym_key: my_pk.to_string(),
            route_blob: my_route,
            muted: false,
            deafened: false,
            display_name: my_name,
        });
    }
    entries
}

pub(in crate::signaling) fn handle_voice_roster(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: String,
    participants: Vec<VoiceRosterEntry>,
) {
    // §10.1/§10.5 — surface the full roster to the frontend so a joiner
    // immediately sees everyone already present, decoupled from
    // MEK-decrypt. The packet path only ever revealed members whose
    // audio we could decrypt.
    deps.emit_event(CommunityVoiceEvent::VoiceRoster {
        community_id: community_id.to_string(),
        channel_id: channel_id.clone(),
        participants: participants
            .iter()
            .map(|e| crate::signaling::deps::VoiceRosterParticipant {
                pseudonym_key: e.pseudonym_key.clone(),
                display_name: e.display_name.clone(),
            })
            .collect(),
    });

    // §10.6 channel scoping — only absorb roster routes for the
    // channel our engine is bound to. Rosters for other channels are
    // UI-only; adding their peers would cross-wire two channels'
    // media planes.
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let deps_task = Arc::clone(deps);
    let cid = community_id.to_string();
    let my_pk = deps.my_pseudonym(community_id).unwrap_or_default();
    let handle = tokio::spawn(async move {
        let (added, remote_count) = {
            let mut t = transport.lock().await;
            let mut added: Vec<(String, Option<String>)> = Vec::new();
            for entry in &participants {
                if !entry.route_blob.is_empty()
                    && entry.pseudonym_key != my_pk
                    && t.add_peer(
                        &entry.pseudonym_key,
                        &entry.route_blob,
                        entry.display_name.as_deref(),
                    )
                {
                    added.push((entry.pseudonym_key.clone(), entry.display_name.clone()));
                }
            }
            (added, t.peer_count())
        };
        for (pseudonym_key, display_name) in added {
            deps_task.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
                community_id: cid.clone(),
                channel_id: channel_id.clone(),
                pseudonym_key,
                present: true,
                display_name,
                remote_count,
            });
        }
        // We (the joiner) just learned the channel roster — directed
        // re-advertise so every present member gets our caps. The
        // session-start advertisement ran against an empty roster.
        deps_task.advertise_media_capabilities(&cid, &channel_id);
        // Roster receipt is leg-2 evidence too: a member built and
        // sent it because they saw our VoiceJoin. Advance + confirm.
        if transport.lock().await.advance_handshake_seen() {
            deps_task.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
                community_id: cid.clone(),
                channel_id: channel_id.clone(),
                state: "seen".to_string(),
                peer: None,
                display_name: None,
            });
        }
        send_confirmed_if_first(&*deps_task, &cid, &channel_id, &transport).await;
    });
    deps.register_background_handle(handle);
}

#[cfg(test)]
mod tests {
    use super::build_roster_entries;

    #[test]
    fn roster_includes_self_with_empty_transport() {
        // The decoupled-roster fix hinges on this: a present member's
        // broadcast must carry itself, even before any peer is in its
        // transport, or a later joiner never learns about it.
        let entries =
            build_roster_entries("me", vec![1, 2, 3], Some("Me Name".to_string()), vec![]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].pseudonym_key, "me");
        assert_eq!(entries[0].route_blob, vec![1, 2, 3]);
        assert_eq!(entries[0].display_name.as_deref(), Some("Me Name"));
    }

    #[test]
    fn roster_appends_self_after_peers() {
        let peers = vec![("peerA".to_string(), vec![9], Some("Peer A".to_string()))];
        let entries = build_roster_entries("me", vec![1], None, peers);
        assert_eq!(entries.len(), 2);
        let peer_a = entries
            .iter()
            .find(|e| e.pseudonym_key == "peerA")
            .expect("peerA present");
        assert_eq!(peer_a.display_name.as_deref(), Some("Peer A"));
        assert!(entries.iter().any(|e| e.pseudonym_key == "me"));
    }

    #[test]
    fn roster_does_not_duplicate_self_already_in_transport() {
        let peers = vec![("me".to_string(), vec![7], None)];
        let entries = build_roster_entries("me", vec![1], None, peers);
        assert_eq!(entries.len(), 1);
        // transport route is authoritative — self is not re-appended
        assert_eq!(entries[0].route_blob, vec![7]);
    }
}
