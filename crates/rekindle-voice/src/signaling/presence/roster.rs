//! Roster assembly and the `VoiceRoster` handler: building the peer list
//! a present member broadcasts to a joiner, and absorbing an inbound
//! roster into the local transport.

use std::sync::Arc;

use rekindle_types::key_format::pseudonym_hex;
use rekindle_types::subscription_events::JoinHandshake;

use rekindle_codec::community::envelope::{MediaSessionKey, VoiceRosterEntry};

use crate::signaling::deps::{CommunityVoiceEvent, VoiceRosterParticipant, VoiceSignalingDeps};
use crate::signaling::media_keys;
use crate::transport::VoiceTransport;

use super::join::{advance_handshake, send_confirmed_if_first};

/// Project transport membership into a roster with the Host's current names.
pub(in crate::signaling) fn named_roster(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    mut members: Vec<String>,
) -> Vec<VoiceRosterParticipant> {
    members.sort_unstable();
    members
        .into_iter()
        .map(|pseudonym_key| VoiceRosterParticipant {
            display_name: deps.member_shown_name(community_id, &pseudonym_key),
            pseudonym_key,
        })
        .collect()
}

/// Build the roster a present member broadcasts to a joiner. Routes
/// come straight from the transport (the route each peer advertised in
/// its VoiceJoin — authoritative for voice), and SELF is appended with
/// `our_media_route_blob()` so the joiner learns about the broadcaster too —
/// without self, a member that joined first would never appear in a
/// later joiner's roster.
pub(super) async fn roster_entries(
    deps: &dyn VoiceSignalingDeps,
    community_id: &str,
    channel_id: &str,
    my_pk: &str,
    transport: &Arc<tokio::sync::Mutex<VoiceTransport>>,
) -> Vec<VoiceRosterEntry> {
    let peers = transport.lock().await.peer_entries();
    let peers = peers
        .into_iter()
        .map(|(key, route)| {
            let name = deps.member_shown_name(community_id, &key);
            (key, route, Some(name))
        })
        .collect();
    let mut entries = build_roster_entries(
        my_pk,
        deps.our_media_route_blob(),
        deps.my_display_name(),
        peers,
    );
    // Each entry carries its participant's session key as it announced
    // it, so a joiner can seal to everyone from the roster (plan C7.29).
    for entry in &mut entries {
        entry.session_key = if entry.pseudonym_key == my_pk {
            media_keys::our_session_key(deps, community_id, channel_id)
        } else {
            media_keys::relayed_session_key(deps, community_id, channel_id, &entry.pseudonym_key)
        };
    }
    entries
}

/// Pure roster assembly: map the transport's `(pseudonym, route, name)`
/// peers to entries, then append SELF (deduped) with `my_route` +
/// `my_name`. Self must be present or a member that joined first never
/// appears in a later joiner's roster. Extracted from `roster_entries`
/// so the self-inclusion invariant is unit-testable without a deps mock.
fn build_roster_entries(
    my_pk: &str,
    my_route: Option<Vec<u8>>,
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
                session_key: MediaSessionKey::default(),
            },
        )
        .collect();

    // Without a media route there is nothing to reach us by, so self is
    // left out rather than advertised with an empty route.
    let my_route = my_route.filter(|_| !my_pk.is_empty());
    if let Some(my_route) = my_route.filter(|_| !entries.iter().any(|e| e.pseudonym_key == my_pk)) {
        entries.push(VoiceRosterEntry {
            pseudonym_key: my_pk.to_string(),
            route_blob: my_route,
            muted: false,
            deafened: false,
            display_name: my_name,
            session_key: MediaSessionKey::default(),
        });
    }
    entries
}

pub(in crate::signaling) fn handle_voice_roster(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    channel_id: String,
    mut participants: Vec<VoiceRosterEntry>,
) {
    participants.retain(|entry| pseudonym_hex(&entry.pseudonym_key).is_ok());
    // §10.1/§10.5 — surface the full roster to the frontend so a joiner
    // immediately sees everyone already present, decoupled from
    // MEK-decrypt. The packet path only ever revealed members whose
    // audio we could decrypt.
    deps.emit_event(CommunityVoiceEvent::VoiceRoster {
        community_id: community_id.to_string(),
        channel_id: channel_id.clone(),
        participants: named_roster(
            &**deps,
            community_id,
            participants
                .iter()
                .map(|e| e.pseudonym_key.clone())
                .collect(),
        ),
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
    deps.scope()
        .spawn_or_drop("voice roster apply", async move {
            let (added, members) = {
                let mut t = transport.lock().await;
                let mut added = Vec::new();
                for entry in &participants {
                    if !entry.route_blob.is_empty()
                        && entry.pseudonym_key != my_pk
                        && t.add_peer(&entry.pseudonym_key, &entry.route_blob)
                    {
                        added.push(entry.pseudonym_key.clone());
                    }
                }
                (added, t.peer_keys())
            };
            let members = named_roster(&*deps_task, &cid, members);
            for pseudonym_key in added {
                // Plan C7.20 — every roster add hands the peer our key,
                // as the join and ack paths do.
                crate::signaling::media_keys::on_peer_added(
                    &deps_task,
                    &cid,
                    &channel_id,
                    &transport,
                    &pseudonym_key,
                )
                .await;
                deps_task.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
                    community_id: cid.clone(),
                    channel_id: channel_id.clone(),
                    display_name: deps_task.member_shown_name(&cid, &pseudonym_key),
                    pseudonym_key,
                    present: true,
                    roster: members.clone(),
                });
            }
            // We (the joiner) just learned the channel roster — directed
            // re-advertise so every present member gets our caps. The
            // session-start advertisement ran against an empty roster.
            deps_task.advertise_media_capabilities(&cid, &channel_id);
            // Roster receipt is leg-2 evidence too: a member built and
            // sent it because they saw our VoiceJoin. Advance + confirm.
            advance_handshake(
                &*deps_task,
                &cid,
                &channel_id,
                &transport,
                JoinHandshake::Seen,
                None,
            )
            .await;
            send_confirmed_if_first(&*deps_task, &cid, &channel_id, &transport).await;
        });
}

#[cfg(test)]
#[path = "roster_tests.rs"]
mod tests;
