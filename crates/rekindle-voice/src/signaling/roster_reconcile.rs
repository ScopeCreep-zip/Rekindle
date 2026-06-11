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
/// (`(pseudonym, seconds-since-added)`) and the scan's presence view.
///
/// - **Add**: fresh row claiming OUR channel, not yet in the roster,
///   not us, with a usable route blob.
/// - **Remove**: roster entry past [`JOIN_GRACE_SECS`] whose presence
///   row either freshly claims a different/no channel (definitive
///   leave) or has gone heartbeat-stale (vanished client — the
///   MatrixRTC `expires` analog).
/// - **Keep**: anyone with no presence row at all — a scan miss must
///   never kick a live peer off the media plane.
#[must_use]
pub fn compute_roster_reconcile(
    bound_channel: &str,
    my_pseudonym: &str,
    roster: &[(String, u64)],
    presence: &[PresencePeerView],
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();
    let in_roster: std::collections::HashSet<&str> =
        roster.iter().map(|(k, _)| k.as_str()).collect();

    for row in presence {
        let claims_our_channel = row.voice_channel_id.as_deref() == Some(bound_channel);
        if row.fresh
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

    let plan = {
        let t = transport.lock().await;
        compute_roster_reconcile(&channel_id, &my_pk, &t.peer_views(), &rows)
    };
    if plan.add.is_empty() && plan.remove.is_empty() {
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
mod tests {
    use super::*;

    fn row(pseudonym: &str, channel: Option<&str>, fresh: bool) -> PresencePeerView {
        PresencePeerView {
            pseudonym_hex: pseudonym.to_string(),
            display_name: Some(format!("{pseudonym}-name")),
            route_blob: vec![1, 2, 3],
            voice_channel_id: channel.map(str::to_string),
            fresh,
        }
    }

    #[test]
    fn fresh_claim_for_our_channel_is_added() {
        let plan = compute_roster_reconcile("ch1", "me", &[], &[row("alice", Some("ch1"), true)]);
        assert_eq!(plan.add.len(), 1);
        assert_eq!(plan.add[0].pseudonym_hex, "alice");
        assert_eq!(plan.add[0].display_name.as_deref(), Some("alice-name"));
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn self_other_channel_stale_and_routeless_rows_are_not_added() {
        let mut routeless = row("dave", Some("ch1"), true);
        routeless.route_blob.clear();
        let plan = compute_roster_reconcile(
            "ch1",
            "me",
            &[],
            &[
                row("me", Some("ch1"), true),     // self
                row("bob", Some("ch2"), true),    // different channel
                row("carol", Some("ch1"), false), // stale heartbeat
                row("erin", None, true),          // not in any channel
                routeless,                        // no route yet
            ],
        );
        assert!(plan.add.is_empty());
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn already_rostered_member_is_not_readded() {
        let roster = vec![("alice".to_string(), 100u64)];
        let plan =
            compute_roster_reconcile("ch1", "me", &roster, &[row("alice", Some("ch1"), true)]);
        assert!(plan.add.is_empty());
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn join_grace_protects_fresh_roster_entries() {
        // alice's row freshly says she left — but she was added 10s
        // ago via gossip; her presence row may simply predate her join.
        let roster = vec![("alice".to_string(), 10u64)];
        let plan = compute_roster_reconcile("ch1", "me", &roster, &[row("alice", None, true)]);
        assert!(plan.remove.is_empty());
    }

    #[test]
    fn fresh_row_claiming_elsewhere_expires_aged_entry() {
        let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
        let plan =
            compute_roster_reconcile("ch1", "me", &roster, &[row("alice", Some("ch2"), true)]);
        assert_eq!(plan.remove, vec!["alice".to_string()]);
    }

    #[test]
    fn stale_heartbeat_expires_aged_entry() {
        let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
        let plan =
            compute_roster_reconcile("ch1", "me", &roster, &[row("alice", Some("ch1"), false)]);
        assert_eq!(plan.remove, vec!["alice".to_string()]);
    }

    #[test]
    fn missing_presence_row_never_expires_a_peer() {
        let roster = vec![("alice".to_string(), 10_000u64)];
        let plan = compute_roster_reconcile("ch1", "me", &roster, &[]);
        assert!(plan.remove.is_empty());
    }
}
