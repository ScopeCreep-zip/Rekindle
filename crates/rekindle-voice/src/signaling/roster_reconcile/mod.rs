//! Presence-derived voice roster reconcile — the three-path Path-1
//! backstop (MatrixRTC `m.rtc.member` pattern mapped onto SMPL
//! presence rows).
//!
//! Gossip `VoiceJoin`/`VoiceLeave` is the fast path; each member's
//! presence row carries a cleartext `voice_channel_id` claim renewed by
//! the heartbeat. After every registry scan the orchestrator hands the
//! presence-derived membership view here and the roster converges from
//! durable state: a member whose row claims our channel but who is not
//! in our roster (one of the two join messages was lost) is introduced
//! to by re-sending our `VoiceJoin`, and ghosts whose rows say "left"
//! (or whose heartbeat went stale) get expired. Pure decision in
//! [`compute_roster_reconcile`]; application + UI events in
//! [`reconcile_from_presence`].
//!
//! **The row is the directory, never the connection.** It carries no
//! media route (plan C7.15): the route is connection data for a call
//! and rides the call's own signaling, as Discord's
//! `VOICE_SERVER_UPDATE` and Jingle's `transport-info` do. So a repair
//! re-runs the join handshake instead of adding a peer from its row: our
//! `VoiceJoin` makes the peer add us and ack, and the ack (carrying its
//! route) adds the peer here (`presence::ack`).
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
//! whose peer is streaming to us still qualifies for the repair. When a peer really
//! leaves, media stops within seconds and the next scan expires them
//! normally.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};

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
    /// The member's voice channel claim, if any.
    pub voice_channel_id: Option<String>,
    /// Row passed the scan's liveness gate (fresh heartbeat +
    /// non-offline status).
    pub fresh: bool,
}

/// The reconcile decision: who to introduce ourselves to, who to expire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcilePlan {
    /// Members in our channel by their rows but not in our roster.
    pub introduce: Vec<String>,
    pub remove: Vec<String>,
}

/// Pure reconcile decision over the transport roster
/// (`(pseudonym, seconds-since-added)`), the scan's presence view, and
/// the media plane's live-peer set (see the module doc: media outranks
/// the directory).
///
/// - **Introduce**: row claiming OUR channel that is fresh OR media-live
///   (a stale row whose peer is streaming to us is alive — the row is
///   stale because their DHT writes fail), not yet in the roster, not us.
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
        {
            plan.introduce.push(row.pseudonym_hex.clone());
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

/// Reciprocity retry — rostered peers who should be hearing our media
/// but send NONE back to us (not media-live), while their presence row
/// is fresh and claims OUR channel.
///
/// Not receiving reciprocal media from a peer that is present and in our
/// channel is the signal that OUR route never reached them: our
/// `VoiceJoin`/`VoiceJoinAck` was a fire-and-forget send with no ack, so
/// a single lost leg leaves us added-to-them (we send, they receive) but
/// them-not-added-to-us (they can't send back) — a persistent one-way.
/// Re-announce our route to them each reconcile until media flows back,
/// which stops the retry (a peer streaming to us already holds our
/// route). This is the RFC 3550 receiver-report / RFC 7675 consent
/// principle applied with the signals we have: reciprocal media is the
/// proof the path is two-way.
///
/// Disjoint from [`compute_roster_reconcile`]'s introductions (those are
/// `!in_roster`; these are in-roster) and never targets a media-live
/// peer or ourselves.
#[must_use]
pub fn compute_route_reannounce<S: std::hash::BuildHasher>(
    bound_channel: &str,
    my_pseudonym: &str,
    roster: &[(String, u64)],
    presence: &[PresencePeerView],
    media_live: &std::collections::HashSet<String, S>,
) -> Vec<String> {
    let in_roster: std::collections::HashSet<&str> =
        roster.iter().map(|(k, _)| k.as_str()).collect();
    presence
        .iter()
        .filter(|row| {
            in_roster.contains(row.pseudonym_hex.as_str())
                && !media_live.contains(row.pseudonym_hex.as_str())
                && row.fresh
                && row.voice_channel_id.as_deref() == Some(bound_channel)
                && row.pseudonym_hex != my_pseudonym
        })
        .map(|row| row.pseudonym_hex.clone())
        .collect()
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
    let (plan, reannounce) = {
        let t = transport.lock().await;
        let roster = t.peer_views();
        let plan = compute_roster_reconcile(&channel_id, &my_pk, &roster, &rows, &media_live);
        let reannounce = compute_route_reannounce(&channel_id, &my_pk, &roster, &rows, &media_live);
        (plan, reannounce)
    };
    if plan.introduce.is_empty() && plan.remove.is_empty() && reannounce.is_empty() {
        return;
    }

    let (removed, remote_count) = {
        let mut t = transport.lock().await;
        let removed: Vec<bool> = plan.remove.iter().map(|gone| t.remove_peer(gone)).collect();
        (removed, t.peer_count())
    };
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
            // Plan C7.20 — an expired peer is a departure: rotate.
            let is_stage = deps
                .stage_channel_info(community_id, &channel_id)
                .is_some_and(|s| s.is_stage);
            crate::signaling::media_keys::on_peer_removed(
                deps,
                community_id,
                &channel_id,
                &transport,
                gone,
                is_stage,
            )
            .await;
        }
    }

    // Both repairs deliver our media route; without one there is nothing
    // to introduce us by.
    let Some(our_route) = deps.our_media_route_blob() else {
        return;
    };

    // Lost-join repair: re-run the join handshake. A peer that missed our
    // `VoiceJoin` adds us from it and acks with its own route, which adds
    // it here (`presence::ack`). Gossip dedups a `Control` envelope by its
    // content, so peers that already applied this join drop the resend.
    // Repeats every scan until the peer is in our roster.
    if !plan.introduce.is_empty() {
        let join = CommunityEnvelope::Control(ControlPayload::VoiceJoin {
            channel_id: channel_id.clone(),
            route_blob: our_route.clone(),
            display_name: deps.my_display_name(),
        });
        deps.send_to_mesh(community_id, &join);
        tracing::info!(
            community = %community_id,
            channel = %channel_id,
            peers = ?plan.introduce,
            "presence reconcile: re-sent our voice join to members missing from our roster",
        );
    }

    // Reciprocity retry — re-deliver our route to rostered peers who are
    // sending us no media (see `compute_route_reannounce`). A directed
    // VoiceJoinAck with `joiner_pseudonym` = the target's own key is
    // exactly what the join path uses; the peer treats it as "the ack
    // that is for me" and adds us with our route (`presence::ack`).
    // Idempotent (a route upsert), fire-and-forget, self-limiting: once
    // the peer holds our route and streams back, media_live drops them
    // from this set.
    for peer in &reannounce {
        let ack = CommunityEnvelope::Control(ControlPayload::VoiceJoinAck {
            channel_id: channel_id.clone(),
            joiner_pseudonym: peer.clone(),
            display_name: deps.my_display_name(),
            route_blob: our_route.clone(),
        });
        deps.send_to_channel(community_id, &channel_id, &ack);
        tracing::info!(
            community = %community_id,
            channel = %channel_id,
            peer = %peer,
            "presence reconcile: re-announced our route to a silent rostered peer (reciprocity retry)",
        );
    }
}

#[cfg(test)]
mod tests;
