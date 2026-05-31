//! Phase 18.g — join-stage primitives.
//!
//! Ported from `src-tauri/src/services/community/join/flow.rs`. Heavy
//! crate-side helpers consumed by src-tauri's `join_community`
//! orchestrator (chiral split — see `join.rs` module docs).
//!
//! Hosts the multi-segment governance snapshot loader, the slot-claim
//! state machine, and the Plate Gate auto-expand-and-retry path.

use rekindle_governance::invite_quota;
use rekindle_governance::merge;
use rekindle_governance::permissions::compute_permissions;
use rekindle_governance::state::{GovernanceState, SegmentState};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MANAGE_COMMUNITY;
use rekindle_types::presence::MemberPresence;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::join::{default_community_name, merge_presence_entry, InitialPresence};
use crate::segments;

/// Snapshot of the multi-segment governance record set, ready for the
/// adapter to build a fresh `CommunityState`.
pub struct GovernanceSnapshot {
    pub all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    pub gov_state: GovernanceState,
    pub name: String,
    pub description: Option<String>,
}

/// Outcome of a successful slot claim — segment + local subkey within
/// that segment + the (string-formatted) writer keypair used for future
/// writes. The adapter persists this into `CommunityState`.
pub struct ClaimedSlot {
    pub registry_key: String,
    pub segment_index: u32,
    pub local_subkey: u32,
    pub slot_keypair_str: String,
}

/// First-pass + multi-segment governance snapshot: fetch + W26-verify
/// every signed `GovernanceSubkeyPayload` from the primary record, merge
/// to find announced segments, fetch + verify each segment, re-merge.
pub async fn load_governance_snapshot<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<GovernanceSnapshot, GovernanceRuntimeError> {
    let mut all_entries = fetch_governance_record_entries(deps, governance_key_str).await?;
    let gov_state_v1 = merge::merge(&all_entries);

    // Pass 2: fetch every segment's governance record. CRDT idempotence +
    // commutativity (Almeida 2016 §3) means re-merge is canonical
    // regardless of fetch order.
    for segment in &gov_state_v1.segments {
        if segment.segment_index == 0 {
            continue;
        }
        match fetch_governance_record_entries(deps, &segment.governance_key).await {
            Ok(mut extra) => all_entries.append(&mut extra),
            Err(e) => tracing::warn!(
                segment = segment.segment_index,
                governance_key = %segment.governance_key,
                error = %e,
                "load_governance_snapshot: skipping unreachable segment governance record"
            ),
        }
    }
    let gov_state = if gov_state_v1.segments.iter().any(|s| s.segment_index > 0) {
        merge::merge(&all_entries)
    } else {
        gov_state_v1
    };

    let name = gov_state.metadata.as_ref().map_or_else(
        || default_community_name(governance_key_str),
        |metadata| metadata.name.clone(),
    );
    let description = gov_state
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.description.clone());

    Ok(GovernanceSnapshot {
        all_entries,
        gov_state,
        name,
        description,
    })
}

/// Scan all 255 subkeys of a single governance record. W26 — verify each
/// payload's pseudonym signature; reject unsigned / mis-signed entries
/// (any member could write to any slot, signature is the only authorship
/// proof).
async fn fetch_governance_record_entries<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<Vec<(PseudonymKey, Vec<GovernanceEntry>)>, GovernanceRuntimeError> {
    deps.open_dht_record(governance_key_str, None).await?;
    let mut all_entries = Vec::new();
    for subkey in 0..255u32 {
        let Ok(Some(bytes)) = deps.get_dht_value(governance_key_str, subkey, false).await else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let Ok(payload) = serde_json::from_slice::<GovernanceSubkeyPayload>(&bytes) else {
            continue;
        };
        let Ok(sig_arr): Result<[u8; 64], _> = payload.signature.as_slice().try_into() else {
            continue;
        };
        if derive::verify_pseudonym_signature(
            &payload.author_pseudonym.0,
            &payload.signing_bytes(),
            &sig_arr,
        )
        .is_err()
        {
            continue;
        }
        all_entries.push((payload.author_pseudonym, payload.entries));
    }
    Ok(all_entries)
}

/// (segment_index, registry_key, slot_range_start) for each segment to
/// try in claim order. Segment 0 is the inviter's registry; later
/// segments come from merged governance's `SegmentAdded` entries.
struct SegmentClaimCandidate {
    segment_index: u32,
    registry_key: String,
    slot_range_start: u32,
}

/// Inner outcome of one slot-claim attempt sweep over the candidate list.
struct ClaimAttemptOutcome {
    claimed: Option<ClaimedSlot>,
    last_full_segment: Option<u32>,
}

/// Claim a registry slot for the joiner. Tries segment 0 first, then
/// every additional segment merged from governance state. If all
/// segments are full, delegates to `auto_expand_and_retry` (P4.3 Plate
/// Gate).
///
/// M10.3 — joiner-side invite quota check runs before any slot write as
/// defense-in-depth (reader-validates also drops over-quota
/// `InviteCreated` entries at merge time).
#[allow(clippy::too_many_arguments, reason = "Mirrors src-tauri signature; refactor into a context struct would propagate to every Phase 18 callsite without simplifying.")]
pub async fn claim_registry_slot<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    invite_registry_key: &str,
    slot_seed_hex: &str,
    inviter_pseudonym: &PseudonymKey,
    my_pseudo: &PseudonymKey,
    pseudonym_signing: &SigningKey,
    gov_state: &GovernanceState,
    join_status_label: &str,
    display_name: Option<String>,
) -> Result<ClaimedSlot, GovernanceRuntimeError> {
    if !invite_quota::check_active_invites_cap(gov_state, inviter_pseudonym) {
        return Err(GovernanceRuntimeError::Adapter(
            "invite quota exceeded for inviter — community is rate-limiting joins".into(),
        ));
    }

    let slot_seed_bytes: [u8; 32] = hex::decode(slot_seed_hex)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("invalid slot seed hex: {e}")))?
        .try_into()
        .map_err(|_| GovernanceRuntimeError::Crypto("slot seed must be 32 bytes".into()))?;

    let outcome = try_claim_in_candidates(
        deps,
        invite_registry_key,
        &slot_seed_bytes,
        &gov_state.segments,
        my_pseudo,
        pseudonym_signing,
        join_status_label,
        display_name.clone(),
    )
    .await?;
    if let Some(claimed) = outcome.claimed {
        return Ok(claimed);
    }

    if let Some(full_seg) = outcome.last_full_segment {
        return auto_expand_and_retry(
            deps,
            community_id,
            invite_registry_key,
            &slot_seed_bytes,
            my_pseudo,
            pseudonym_signing,
            full_seg,
            gov_state,
            join_status_label,
            display_name,
        )
        .await;
    }
    Err(GovernanceRuntimeError::Adapter(
        "No reachable segment registry — Veilid attach may have failed".into(),
    ))
}

#[allow(clippy::too_many_arguments, reason = "Mirrors src-tauri inner-loop signature; passing a context struct would still need every field at the call site.")]
async fn try_claim_in_candidates<D: GovernanceRuntimeDeps>(
    deps: &D,
    invite_registry_key: &str,
    slot_seed_bytes: &[u8; 32],
    governance_segments: &[SegmentState],
    my_pseudo: &PseudonymKey,
    pseudonym_signing: &SigningKey,
    join_status_label: &str,
    display_name: Option<String>,
) -> Result<ClaimAttemptOutcome, GovernanceRuntimeError> {
    let mut candidates: Vec<SegmentClaimCandidate> = Vec::new();
    candidates.push(SegmentClaimCandidate {
        segment_index: 0,
        registry_key: invite_registry_key.to_string(),
        slot_range_start: 0,
    });
    for seg in governance_segments {
        if seg.segment_index == 0 {
            continue;
        }
        candidates.push(SegmentClaimCandidate {
            segment_index: seg.segment_index,
            registry_key: seg.registry_key.clone(),
            slot_range_start: seg.slot_range_start,
        });
    }
    candidates.sort_by_key(|c| c.segment_index);

    let mut last_full_segment: Option<u32> = None;
    for candidate in &candidates {
        if let Err(e) = deps.open_dht_record(&candidate.registry_key, None).await {
            tracing::warn!(
                segment = candidate.segment_index,
                error = %e,
                "claim_registry_slot: failed to open segment registry — skipping"
            );
            continue;
        }
        let seqs = deps
            .inspect_dht_record_update_get_seqs(&candidate.registry_key)
            .await?;
        let Some(local_subkey) =
            (0..255u32).find(|subkey| seqs.get(*subkey as usize).copied().unwrap_or(0) == 0)
        else {
            last_full_segment = Some(candidate.segment_index);
            continue;
        };

        let global_slot = candidate.slot_range_start + local_subkey;
        let slot_kp = derive::derive_slot_keypair(slot_seed_bytes, global_slot)
            .map_err(|e| GovernanceRuntimeError::Crypto(format!("slot keypair derivation failed: {e}")))?;
        let slot_kp_str = deps.format_writer_keypair(
            slot_kp.verifying_key().to_bytes(),
            slot_kp.to_bytes(),
        );

        let mut presence = MemberPresence {
            pseudonym_key: my_pseudo.clone(),
            display_name: display_name.clone(),
            status: join_status_label.into(),
            route_blob: vec![],
            last_heartbeat: rekindle_utils::timestamp_secs(),
            ..Default::default()
        };
        let presence_sig =
            derive::sign_with_pseudonym(pseudonym_signing, &presence.signing_bytes());
        presence.signature = presence_sig.to_vec();
        let presence_bytes = serde_json::to_vec(&presence).map_err(|e| {
            GovernanceRuntimeError::Encoding(format!("presence serialization failed: {e}"))
        })?;
        let write_outcome = deps
            .set_dht_value(
                &candidate.registry_key,
                local_subkey,
                presence_bytes,
                Some(slot_kp_str.clone()),
            )
            .await?;
        if let Some(stale) = write_outcome {
            return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
        }

        let verify_bytes = deps
            .get_dht_value(&candidate.registry_key, local_subkey, true)
            .await?
            .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
        let written: MemberPresence = serde_json::from_slice(&verify_bytes).map_err(|e| {
            GovernanceRuntimeError::Encoding(format!("slot read-back deserialization failed: {e}"))
        })?;
        if written.pseudonym_key != *my_pseudo {
            return Err(GovernanceRuntimeError::Adapter(format!(
                "Slot collision in segment {} — another member claimed this slot. Please retry.",
                candidate.segment_index
            )));
        }

        return Ok(ClaimAttemptOutcome {
            claimed: Some(ClaimedSlot {
                registry_key: candidate.registry_key.clone(),
                segment_index: candidate.segment_index,
                local_subkey,
                slot_keypair_str: slot_kp_str,
            }),
            last_full_segment,
        });
    }

    Ok(ClaimAttemptOutcome {
        claimed: None,
        last_full_segment,
    })
}

#[allow(clippy::too_many_arguments, reason = "Mirrors src-tauri inner-loop signature; the underlying state is the join cursor itself.")]
async fn auto_expand_and_retry<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    invite_registry_key: &str,
    slot_seed_bytes: &[u8; 32],
    my_pseudo: &PseudonymKey,
    pseudonym_signing: &SigningKey,
    full_segment_index: u32,
    gov_state: &GovernanceState,
    join_status_label: &str,
    display_name: Option<String>,
) -> Result<ClaimedSlot, GovernanceRuntimeError> {
    let perms = compute_permissions(my_pseudo, None, gov_state, rekindle_utils::timestamp_secs());
    let have_manage_community = (perms & MANAGE_COMMUNITY) != 0;
    let requester_pseudonym = hex::encode(my_pseudo.0);

    deps.emit_event(GovernanceRuntimeEvent::JoinPendingAlert {
        have_manage_community,
    });

    if have_manage_community {
        tracing::info!(
            community = %community_id,
            full_segment_index,
            "Plate Gate: joiner has MANAGE_COMMUNITY — expanding inline"
        );
        segments::expand_community_segment(deps, community_id)
            .await
            .map_err(|e| {
                GovernanceRuntimeError::Adapter(format!("inline segment expansion failed: {e}"))
            })?;
    } else {
        tracing::info!(
            community = %community_id,
            full_segment_index,
            "Plate Gate: joiner lacks MANAGE_COMMUNITY — gossiping RequestSegmentExpansion"
        );
        let envelope = CommunityEnvelope::Control(ControlPayload::RequestSegmentExpansion {
            community_id: community_id.to_string(),
            requester_pseudonym,
            full_segment_index,
        });
        deps.send_to_mesh(community_id, &envelope)?;
    }

    // Poll up to 30s for the new segment to land in our merged
    // governance state. Quicker than a watch-driven notify, simpler than
    // a Notify channel, and bounded.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut new_segments: Vec<SegmentState>;
    loop {
        new_segments = deps
            .governance_state(community_id)
            .map(|gov| gov.segments)
            .unwrap_or_default();
        let max_seg = new_segments
            .iter()
            .map(|s| s.segment_index)
            .max()
            .unwrap_or(0);
        if max_seg > full_segment_index {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(GovernanceRuntimeError::Adapter(format!(
                "Community is full and no admin expanded within 30s. The community has {} active segment(s); admins must run expand_community_segment to grow it.",
                new_segments.len() + 1
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let outcome = try_claim_in_candidates(
        deps,
        invite_registry_key,
        slot_seed_bytes,
        &new_segments,
        my_pseudo,
        pseudonym_signing,
        join_status_label,
        display_name,
    )
    .await?;
    if let Some(claimed) = outcome.claimed {
        return Ok(claimed);
    }
    Err(GovernanceRuntimeError::Adapter(
        "Community segment expanded but still full — concurrent joiner race; please retry".into(),
    ))
}

/// DHT registry scan + W26-verify each `MemberPresence` row, populating
/// the initial `peers` + `online_members` + `known_members` sets that
/// the adapter then plumbs into `GossipOverlay`.
pub async fn collect_initial_presence_state<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    my_slot: u32,
    my_pseudo_hex: &str,
) -> InitialPresence {
    let mut presence = InitialPresence::default();
    presence.known_members.insert(my_pseudo_hex.to_string());

    for subkey in 0..255u32 {
        if subkey == my_slot {
            continue;
        }
        let Ok(Some(bytes)) = deps.get_dht_value(registry_key, subkey, false).await else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<MemberPresence>(&bytes) else {
            continue;
        };
        let Ok(sig_arr): Result<[u8; 64], _> = row.signature.as_slice().try_into() else {
            continue;
        };
        if derive::verify_pseudonym_signature(
            &row.pseudonym_key.0,
            &row.signing_bytes(),
            &sig_arr,
        )
        .is_err()
        {
            continue;
        }
        let pseudo_hex = hex::encode(row.pseudonym_key.0);
        merge_presence_entry(
            &mut presence,
            &pseudo_hex,
            &row.status,
            &row.route_blob,
            row.last_heartbeat,
        );
    }

    presence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_snapshot_struct_fields_accessible() {
        // Smoke check: GovernanceSnapshot can be constructed + read.
        let snap = GovernanceSnapshot {
            all_entries: Vec::new(),
            gov_state: GovernanceState::default(),
            name: "t".into(),
            description: None,
        };
        assert_eq!(snap.name, "t");
        assert!(snap.all_entries.is_empty());
    }

    #[test]
    fn claimed_slot_carries_keypair_string() {
        let slot = ClaimedSlot {
            registry_key: "rk".into(),
            segment_index: 0,
            local_subkey: 5,
            slot_keypair_str: "kp".into(),
        };
        assert_eq!(slot.registry_key, "rk");
        assert_eq!(slot.local_subkey, 5);
    }
}
