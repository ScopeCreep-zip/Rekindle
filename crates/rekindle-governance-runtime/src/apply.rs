//! Phase 18.c — apply pipeline.
//!
//! Ported from `src-tauri/src/services/community/governance.rs::write_entry`.
//! Pure pipeline: read existing entries → permission validate → DHT
//! write with M9.5 conflict/verify → mesh broadcast → local CRDT merge
//! → UI snapshot emit. All side effects flow through the
//! `GovernanceRuntimeDeps` trait.

use rekindle_governance::{merge, validate};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::derive;
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

/// Architecture §6 — classify a governance entry by which UI snapshot
/// it invalidates so we can emit the right event after a successful
/// CRDT apply. `Roles` triggers a `RolesChanged` snapshot;
/// `ChannelsOrCategories` triggers `ChannelsUpdated`. Returning both
/// false is intentional for entries that touch nothing the UI caches
/// (e.g. lifecycle events handled elsewhere).
#[derive(Default, Clone, Copy)]
struct EntryAffects {
    roles: bool,
    channels_or_categories: bool,
}

fn classify_entry(entry: &GovernanceEntry) -> EntryAffects {
    match entry {
        // Role definition + archive change the role list itself;
        // assignment/unassignment change a member's role_ids and are
        // handled by `MemberRolesChanged` elsewhere.
        GovernanceEntry::RoleDefinition { .. } | GovernanceEntry::RoleArchived { .. } => {
            EntryAffects {
                roles: true,
                ..EntryAffects::default()
            }
        }
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. }
        | GovernanceEntry::CategoryCreated { .. }
        | GovernanceEntry::CategoryArchived { .. }
        | GovernanceEntry::CategoryUpdated { .. } => EntryAffects {
            channels_or_categories: true,
            ..EntryAffects::default()
        },
        _ => EntryAffects::default(),
    }
}

/// Apply a governance entry: sign + write to our SMPL subkey, gossip
/// `GovernanceUpdated`, merge locally, emit UI snapshot event.
///
/// Returns `Ok(())` on success. On M9.5 write conflict or read-back
/// mismatch, the local merge + UI emit is skipped (the entry never
/// landed on the network, so the caller's optimistic UI must roll back).
pub async fn write_entry<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), GovernanceRuntimeError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;

    let my_pseudo_hex = membership
        .my_pseudonym_hex
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_string()))?;
    let pseudo_bytes: [u8; 32] = hex::decode(&my_pseudo_hex)
        .map_err(|e| GovernanceRuntimeError::InvalidPseudonymHex(e.to_string()))?
        .try_into()
        .map_err(|_| {
            GovernanceRuntimeError::InvalidPseudonymHex("pseudonym must be 32 bytes".into())
        })?;
    let pseudo = PseudonymKey(pseudo_bytes);

    if !validate::validate_write(&pseudo, &entry, &gov_state) {
        return Err(GovernanceRuntimeError::PermissionDenied);
    }

    let gov_key_str = membership
        .governance_key
        .ok_or_else(|| GovernanceRuntimeError::GovernanceKeyMissing(community_id.to_string()))?;
    let my_slot = membership
        .my_subkey_index
        .ok_or_else(|| GovernanceRuntimeError::SlotIndexMissing(community_id.to_string()))?;
    let slot_kp_str = membership
        .slot_keypair
        .ok_or_else(|| GovernanceRuntimeError::SlotKeypairMissing(community_id.to_string()))?;

    // Read existing entries from our SMPL subkey. Architecture §26 W26 —
    // re-verify the payload signature before accumulating, so an attacker
    // who overwrote our subkey can't launder forged entries through us.
    let existing_bytes = deps
        .get_dht_value(&gov_key_str, my_slot, false)
        .await?
        .filter(|b| !b.is_empty());
    let mut my_entries: Vec<GovernanceEntry> = existing_bytes
        .as_deref()
        .and_then(|data| serde_json::from_slice::<GovernanceSubkeyPayload>(data).ok())
        .filter(|payload| {
            derive::verify_pseudonym_signature(
                &payload.author_pseudonym.0,
                &payload.signing_bytes(),
                payload.signature.as_slice().try_into().unwrap_or(&[0u8; 64]),
            )
            .is_ok()
        })
        .map(|payload| payload.entries)
        .unwrap_or_default();
    my_entries.push(entry.clone());

    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let pseudonym_signing_key = derive::derive_community_pseudonym(&identity_secret, community_id);

    let mut payload_struct = GovernanceSubkeyPayload {
        author_pseudonym: pseudo.clone(),
        entries: my_entries,
        signature: Vec::new(),
    };
    let signature =
        derive::sign_with_pseudonym(&pseudonym_signing_key, &payload_struct.signing_bytes());
    payload_struct.signature = signature.to_vec();
    let payload = serde_json::to_vec(&payload_struct)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("serialize governance entries: {e}")))?;

    // M9.5 — set_dht_value returns Some(stale) when our write was NOT
    // accepted by the network. Surface as WriteConflict so the caller
    // doesn't emit GovernanceUpdated for a write that didn't land.
    let write_outcome = deps
        .set_dht_value(&gov_key_str, my_slot, payload.clone(), Some(slot_kp_str))
        .await?;
    if let Some(stale) = write_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    // M9.5 — read-back verification. Even on local-set success, network
    // propagation can fail. Force a fresh read and confirm the payload
    // is what the network now serves.
    let verify = deps
        .get_dht_value(&gov_key_str, my_slot, true)
        .await?
        .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
    if verify != payload {
        return Err(GovernanceRuntimeError::VerifyMismatch {
            read: verify.len(),
            written: payload.len(),
        });
    }

    let notification = CommunityEnvelope::Control(ControlPayload::GovernanceUpdated {
        governance_key: gov_key_str,
        subkey_index: my_slot,
        lamport_ts: entry.lamport(),
    });
    deps.send_to_mesh(community_id, &notification)?;

    // Local CRDT merge after the network confirms the entry landed.
    if let Some(mut current_state) = deps.governance_state(community_id) {
        merge::apply_entry(&pseudo, &entry, &mut current_state);
        deps.set_governance_state(community_id, current_state);
    }

    deps.emit_event(GovernanceRuntimeEvent::GovernanceEntryApplied {
        community_id: community_id.to_string(),
        entry: Box::new(entry.clone()),
    });

    let affects = classify_entry(&entry);
    if affects.roles {
        deps.emit_event(GovernanceRuntimeEvent::RolesChanged {
            community_id: community_id.to_string(),
        });
    }
    if affects.channels_or_categories {
        deps.emit_event(GovernanceRuntimeEvent::ChannelsUpdated {
            community_id: community_id.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::ChannelId;

    #[test]
    fn classify_role_definition_marks_roles() {
        let entry = GovernanceEntry::RoleDefinition {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            name: "test".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_role_archived_marks_roles() {
        let entry = GovernanceEntry::RoleArchived {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_channel_created_marks_channels() {
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([0u8; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn classify_category_updated_marks_channels() {
        let entry = GovernanceEntry::CategoryUpdated {
            category_id: rekindle_types::id::CategoryId([0u8; 16]),
            name: Some("renamed".into()),
            position: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn classify_role_assignment_affects_nothing() {
        let entry = GovernanceEntry::RoleAssignment {
            target: PseudonymKey([0u8; 32]),
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(!affects.channels_or_categories);
    }
}
