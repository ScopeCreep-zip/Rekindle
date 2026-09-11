//! Presence-derived voice roster reconcile — the three-path Path-1
//! backstop (MatrixRTC `m.rtc.member` pattern mapped onto SMPL
//! presence rows).
//!
//! Gossip `VoiceJoin`/`VoiceLeave` is the fast path; each member's
//! presence row carries a MEK-encrypted `voice_channel_id` claim
//! renewed by the heartbeat. After every registry scan the orchestrator
//! hands the presence-derived membership view here and the roster
//! converges from durable state: members whose join gossip was lost
//! get added, ghosts whose rows say "left" (or whose heartbeat went
//! stale) get expired. Pure decision in [`compute_roster_reconcile`];
//! application + UI events in [`reconcile_from_presence`].
//!
//! **Media outranks the directory.** In-call liveness is judged on the
//! CALL transport, never on a directory — the principle every shipped
//! voice stack follows (Mumble drops on transport inactivity, Discord's
//! voice gateway on missed voice-connection heartbeats, WebRTC on ICE
//! consent-freshness / RTP inactivity). Presence rows are only the
//! directory: a peer's row goes heartbeat-stale when their DHT presence
//! WRITES fail, which happens routinely because the call itself
//! saturates the Veilid relays. So the reconcile consults the media
//! plane's [`crate::liveness::MediaLiveness`] ledger (accepted voice
//! packets + verified receiver reports, via
//! [`VoiceSignalingDeps::media_live_peers`]): a media-live peer is
//! never evicted regardless of what their row claims, and a stale row
//! whose peer is streaming to us still qualifies for the add repair —
//! the row still carries the route blob we need. When a peer really
//! leaves, media stops within seconds and the next scan expires them
//! normally.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::signaling::deps::{CommunityVoiceEvent, VoiceSignalingDeps};

/// Seconds a roster entry is protected from presence-based removal
/// after being added. A peer who just announced via gossip won't have
/// a propagated presence row for up to a poll cycle (15s write cadence
/// + DHT propagation); expiring them in that window would flap the
/// roster. 45s = three poll cycles of slack.
pub const JOIN_GRACE_SECS: u64 = 45;

/// Presence-derived voice membership view of one community member.
/// Mirrors `rekindle_presence::VoicePresenceRow` — the adapter maps
/// between the two so neither crate depends on the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresencePeerView {
    pub pseudonym_hex: String,
    pub display_name: Option<String>,
    pub route_blob: Vec<u8>,
    /// The member's MEK-decrypted voice channel claim, if any.
    pub voice_channel_id: Option<String>,
    /// Row passed the scan's liveness gate (fresh heartbeat +
    /// non-offline status).
    pub fresh: bool,
}

/// One repaired (lost-gossip) roster addition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileAdd {
    pub pseudonym_hex: String,
    pub route_blob: Vec<u8>,
    pub display_name: Option<String>,
}

/// The reconcile decision: who to add, who to expire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    pub add: Vec<ReconcileAdd>,
    pub remove: Vec<String>,
}

/// Pure reconcile decision over the transport roster
/// (`(pseudonym, seconds-since-added)`), the scan's presence view, and
/// the media plane's live-peer set (see the module doc: media outranks
/// the directory).
///
/// - **Add**: row claiming OUR channel that is fresh OR media-live
///   (a stale row whose peer is streaming to us is alive — the row is
///   stale because their DHT writes fail, and it still carries the
///   route blob), not yet in the roster, not us, with a usable route
///   blob.
/// - **Remove**: roster entry past [`JOIN_GRACE_SECS`], NOT media-live,
///   whose presence row either freshly claims a different/no channel
///   (definitive leave) or has gone heartbeat-stale (vanished client —
///   the MatrixRTC `expires` analog).
/// - **Keep**: any media-live peer (flowing media vetoes both remove
///   branches), and anyone with no presence row at all — a scan miss
///   must never kick a live peer off the media plane.
#[must_use]
pub fn compute_roster_reconcile<S: std::hash::BuildHasher>(
    bound_channel: &str,
    my_pseudonym: &str,
    roster: &[(String, u64)],
    presence: &[PresencePeerView],
    media_live: &std::collections::HashSet<String, S>,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();
    let in_roster: std::collections::HashSet<&str> =
        roster.iter().map(|(k, _)| k.as_str()).collect();

    for row in presence {
        let claims_our_channel = row.voice_channel_id.as_deref() == Some(bound_channel);
        if (row.fresh || media_live.contains(row.pseudonym_hex.as_str()))
            && claims_our_channel
            && !in_roster.contains(row.pseudonym_hex.as_str())
            && row.pseudonym_hex != my_pseudonym
            && !row.route_blob.is_empty()
        {
            plan.add.push(ReconcileAdd {
                pseudonym_hex: row.pseudonym_hex.clone(),
                route_blob: row.route_blob.clone(),
                display_name: row.display_name.clone(),
            });
        }
    }

    for (pseudonym, age_secs) in roster {
        if *age_secs <= JOIN_GRACE_SECS {
            continue;
        }
        // Media veto — covers BOTH remove branches below. Flowing
        // media outranks any presence claim: a stale row means the
        // peer's DHT writes are failing (routine mid-call), and even a
        // fresh row claiming elsewhere loses to packets arriving NOW.
        // When a peer really leaves, media stops within seconds and
        // the next scan expires them normally.
        if media_live.contains(pseudonym.as_str()) {
            continue;
        }
        let Some(row) = presence.iter().find(|r| &r.pseudonym_hex == pseudonym) else {
            continue; // no row — scan miss, keep the peer
        };
        let left = row.fresh && row.voice_channel_id.as_deref() != Some(bound_channel);
        let vanished = !row.fresh;
        if left || vanished {
            plan.remove.push(pseudonym.clone());
        }
    }

    plan
}

/// Send-side route-blob supersession for peers ALREADY in the roster —
/// the heal for a peer who re-announced a new route (the restart case).
///
/// Veilid never reliably marks a peer's old route dead when that peer
/// restarts: the old route persists in their table store and can keep
/// answering our node's background liveness pings, so `dead_remote_routes`
/// never fires and `app_message` (fire-and-forget past hop 1) returns no
/// error while our media dies at the peer's vanished endpoint. The
/// authoritative signal is the peer's own re-published blob: when their
/// presence row carries a route blob that differs from the one the
/// transport is currently sending to, adopt it. Keyed by PEER (pseudonym)
/// and compared by blob bytes — never by blob identity, per the Veilid
/// source dig (a stale RouteId must be superseded, not re-imported).
///
/// This complements [`compute_roster_reconcile`], which only ADDS peers
/// not yet in the roster (its `!in_roster` gate skips exactly the peers
/// handled here). A refresh changes only the route we send over, not
/// roster membership, so it emits no join/leave.
#[must_use]
pub fn compute_blob_refreshes<S: std::hash::BuildHasher>(
    bound_channel: &str,
    my_pseudonym: &str,
    current_blobs: &[(String, Vec<u8>)],
    presence: &[PresencePeerView],
    media_live: &std::collections::HashSet<String, S>,
) -> Vec<ReconcileAdd> {
    let mut refreshes = Vec::new();
    for row in presence {
        let live = row.fresh || media_live.contains(row.pseudonym_hex.as_str());
        let claims_our_channel = row.voice_channel_id.as_deref() == Some(bound_channel);
        if !(live
            && claims_our_channel
            && row.pseudonym_hex != my_pseudonym
            && !row.route_blob.is_empty())
        {
            continue;
        }
        // Only peers we already hold a blob for, and only when it
        // actually changed. A peer not yet rostered is an ADD
        // (`compute_roster_reconcile`), not a refresh.
        if current_blobs
            .iter()
            .any(|(p, cur)| p == &row.pseudonym_hex && cur != &row.route_blob)
        {
            refreshes.push(ReconcileAdd {
                pseudonym_hex: row.pseudonym_hex.clone(),
                route_blob: row.route_blob.clone(),
                display_name: row.display_name.clone(),
            });
        }
    }
    refreshes
}

/// Apply the presence view to the bound voice session: compute the
/// plan over the live transport roster, mutate it, and emit the same
/// `VoiceJoin`/`VoiceLeave` events the gossip path emits so every
/// frontend converges identically. No-op when no voice session is
/// bound to this community.
pub async fn reconcile_from_presence(
    deps: &Arc<dyn VoiceSignalingDeps>,
    community_id: &str,
    rows: Vec<PresencePeerView>,
) {
    let Some(channel_id) = deps.voice_engine_channel_id() else {
        return;
    };
    if !deps.voice_engine_bound_to(community_id, &channel_id) {
        return;
    }
    let Some(transport) = deps.transport_handle() else {
        return;
    };
    let Some(my_pk) = deps.my_pseudonym(community_id) else {
        return;
    };

    let media_live = deps.media_live_peers();
    let (plan, refreshes) = {
        let t = transport.lock().await;
        let current_blobs: Vec<(String, Vec<u8>)> = t
            .peer_named_entries()
            .into_iter()
            .map(|(pseudonym, blob, _)| (pseudonym, blob))
            .collect();
        let plan =
            compute_roster_reconcile(&channel_id, &my_pk, &t.peer_views(), &rows, &media_live);
        let refreshes =
            compute_blob_refreshes(&channel_id, &my_pk, &current_blobs, &rows, &media_live);
        (plan, refreshes)
    };
    if plan.add.is_empty() && plan.remove.is_empty() && refreshes.is_empty() {
        return;
    }

    let (added, removed, remote_count) = {
        let mut t = transport.lock().await;
        let mut added: Vec<bool> = Vec::with_capacity(plan.add.len());
        for add in &plan.add {
            added.push(t.add_peer(
                &add.pseudonym_hex,
                &add.route_blob,
                add.display_name.as_deref(),
            ));
        }
        // Route-blob supersession: upsert the peer's new blob so the next
        // broadcast sends over the fresh route. `add_peer` upserts the
        // blob for an existing roster entry (returns false), so this
        // changes only the route, never membership — no join/leave event.
        for refresh in &refreshes {
            t.add_peer(
                &refresh.pseudonym_hex,
                &refresh.route_blob,
                refresh.display_name.as_deref(),
            );
            tracing::info!(
                community = %community_id,
                channel = %channel_id,
                peer = %refresh.pseudonym_hex,
                "presence reconcile: superseded peer route blob (re-announce)",
            );
        }
        let mut removed: Vec<bool> = Vec::with_capacity(plan.remove.len());
        for gone in &plan.remove {
            removed.push(t.remove_peer(gone));
        }
        (added, removed, t.peer_count())
    };

    for (add, newly) in plan.add.iter().zip(&added) {
        tracing::info!(
            community = %community_id,
            channel = %channel_id,
            peer = %add.pseudonym_hex,
            "presence reconcile: repaired lost voice join",
        );
        deps.emit_event(CommunityVoiceEvent::VoiceJoin {
            community_id: community_id.to_string(),
            channel_id: channel_id.clone(),
            pseudonym_key: add.pseudonym_hex.clone(),
            route_blob: add.route_blob.clone(),
            display_name: add.display_name.clone(),
        });
        if *newly {
            deps.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
                community_id: community_id.to_string(),
                channel_id: channel_id.clone(),
                pseudonym_key: add.pseudonym_hex.clone(),
                present: true,
                display_name: add.display_name.clone(),
                remote_count,
            });
        }
    }
    for (gone, was_present) in plan.remove.iter().zip(&removed) {
        tracing::info!(
            community = %community_id,
            channel = %channel_id,
            peer = %gone,
            "presence reconcile: expired ghost voice peer",
        );
        deps.emit_event(CommunityVoiceEvent::VoiceLeave {
            community_id: community_id.to_string(),
            channel_id: channel_id.clone(),
            pseudonym_key: gone.clone(),
        });
        if *was_present {
            deps.emit_event(CommunityVoiceEvent::VoiceRosterChanged {
                community_id: community_id.to_string(),
                channel_id: channel_id.clone(),
                pseudonym_key: gone.clone(),
                present: false,
                display_name: None,
                remote_count,
            });
        }
    }

    if !plan.add.is_empty() {
        // Handshake convergence for the lost-gossip path — the repair
        // mirrors `voice_join_apply` exactly. (1) Directed ack: the
        // SimpleX-style introduction carries OUR identity + route, so
        // the repaired peer adds us from the ack alone and their
        // handshake advances even though our original VoiceJoin never
        // reached them. (2) Mutual evidence: a fresh presence row
        // claiming our channel is the durable Path-1 analog of the
        // mutual VoiceJoin that `voice_join_apply` already counts as
        // leg 2; count it the same way and complete leg 3. Without
        // this, two peers who both missed each other's join gossip sit
        // at `handshake-announced` forever and media-ready never opens.
        for add in &plan.add {
            let ack = CommunityEnvelope::Control(ControlPayload::VoiceJoinAck {
                channel_id: channel_id.clone(),
                joiner_pseudonym: add.pseudonym_hex.clone(),
                display_name: deps.my_display_name(),
                route_blob: deps.our_route_blob(),
            });
            deps.send_to_channel(community_id, &channel_id, &ack);
        }
        if transport.lock().await.advance_handshake_seen() {
            let first = &plan.add[0];
            deps.emit_event(CommunityVoiceEvent::VoiceJoinHandshake {
                community_id: community_id.to_string(),
                channel_id: channel_id.clone(),
                state: "seen".to_string(),
                peer: Some(first.pseudonym_hex.clone()),
                display_name: first.display_name.clone(),
            });
        }
        crate::signaling::presence::send_confirmed_if_first(
            deps.as_ref(),
            community_id,
            &channel_id,
            &transport,
        )
        .await;

        // Repaired members missed every capabilities advertise we made
        // before they appeared — directed re-advertise (receivers dedup),
        // then re-check the mesh→MCU threshold the gossip join path
        // would have evaluated.
        deps.advertise_media_capabilities(community_id, &channel_id);
        crate::signaling::presence::maybe_switch_to_mcu(
            deps.as_ref(),
            community_id,
            &channel_id,
            &transport,
            &my_pk,
        )
        .await;
    }
}

#[cfg(test)]
mod tests;
