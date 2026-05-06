use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::governance::GovernanceEntry;
use tauri::Emitter as _;

use crate::channels::community_channel::{
    ChannelsUpdatedCategoryDto, ChannelsUpdatedChannelDto, CommunityEvent, RoleDto,
};
use crate::state::SharedState;
use crate::state_helpers;

/// Architecture §6 — classify a governance entry by which UI snapshot
/// it invalidates so we can emit the right event after a successful
/// CRDT apply. `Roles` triggers a `RolesChanged` snapshot;
/// `ChannelsOrCategories` triggers `ChannelsUpdated`. Returning both
/// is intentional for entries that semantically touch nothing the UI
/// caches (e.g. lifecycle events handled elsewhere).
#[derive(Default, Clone, Copy)]
struct EntryAffects {
    roles: bool,
    channels_or_categories: bool,
}

fn classify_entry(entry: &GovernanceEntry) -> EntryAffects {
    match entry {
        // Role definition + archive are the only entries that change
        // the role list itself; assignment/unassignment change a
        // member's role_ids and are handled by `MemberRolesChanged`.
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

fn snapshot_roles(state: &SharedState, community_id: &str) -> Vec<RoleDto> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .map(|community| community.roles.iter().map(RoleDto::from).collect())
        .unwrap_or_default()
}

fn snapshot_channels_and_categories(
    state: &SharedState,
    community_id: &str,
) -> (
    Vec<ChannelsUpdatedChannelDto>,
    Vec<ChannelsUpdatedCategoryDto>,
) {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return (Vec::new(), Vec::new());
    };
    let channels = community
        .channels
        .iter()
        .map(|channel| ChannelsUpdatedChannelDto {
            id: channel.id.clone(),
            name: channel.name.clone(),
            channel_type: channel.channel_type.to_string(),
            category_id: channel.category_id.clone(),
            topic: channel.topic.clone(),
            slowmode_seconds: channel.slowmode_seconds,
        })
        .collect();
    let categories = community
        .categories
        .iter()
        .map(|category| ChannelsUpdatedCategoryDto {
            id: category.id.clone(),
            name: category.name.clone(),
            sort_order: category.sort_order,
        })
        .collect();
    (channels, categories)
}

fn emit_community_event(state: &SharedState, event: CommunityEvent) {
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };
    let _ = app_handle.emit("community-event", event);
}

pub async fn write_entry(
    state: &SharedState,
    community_id: &str,
    entry: rekindle_types::governance::GovernanceEntry,
) -> Result<(), String> {
    let gov_state = state_helpers::governance_state(state, community_id)
        .ok_or("governance state not loaded for this community")?;
    let my_pseudo_hex = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };
    let pseudo_bytes: [u8; 32] = hex::decode(&my_pseudo_hex)
        .map_err(|e| format!("invalid pseudonym hex: {e}"))?
        .try_into()
        .map_err(|_| "pseudonym must be 32 bytes")?;
    let pseudo = rekindle_types::id::PseudonymKey(pseudo_bytes);

    if !rekindle_governance::validate::validate_write(&pseudo, &entry, &gov_state) {
        return Err("insufficient permission for this governance operation".into());
    }

    let (gov_key_str, my_slot, slot_kp_str) = {
        let communities = state.communities.read();
        let cs = communities.get(community_id).ok_or("community not found")?;
        (
            cs.governance_key
                .clone()
                .ok_or("no governance key - community not using v2.0 governance")?,
            cs.my_subkey_index.ok_or("no slot index")?,
            cs.slot_keypair.clone().ok_or("no slot keypair")?,
        )
    };

    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let gov_key: veilid_core::RecordKey = gov_key_str
        .parse()
        .map_err(|e| format!("invalid governance key: {e}"))?;
    let slot_kp: veilid_core::KeyPair = slot_kp_str
        .parse()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;

    let mut my_entries: Vec<rekindle_types::governance::GovernanceEntry> =
        match rc.get_dht_value(gov_key.clone(), my_slot, false).await {
            Ok(Some(val)) if !val.data().is_empty() => serde_json::from_slice::<
                rekindle_types::governance::GovernanceSubkeyPayload,
            >(val.data())
            // Architecture §26 W26 — re-verify the existing payload before
            // accumulating into it. Otherwise an attacker who managed to
            // overwrite our subkey via the shared slot_seed would launder
            // their forged entries into every subsequent legitimate write
            // we make.
            .ok()
            .filter(|payload| {
                rekindle_secrets::derive::verify_pseudonym_signature(
                    &payload.author_pseudonym.0,
                    &payload.signing_bytes(),
                    payload.signature.as_slice().try_into().unwrap_or(&[0u8; 64]),
                )
                .is_ok()
            })
            .map(|payload| payload.entries)
            .unwrap_or_default(),
            _ => Vec::new(),
        };
    my_entries.push(entry.clone());

    let identity_secret = {
        let guard = state.identity_secret.lock();
        *guard.as_ref().ok_or("identity secret not available")?
    };
    let pseudonym_signing_key =
        rekindle_secrets::derive::derive_community_pseudonym(&identity_secret, community_id);
    let mut payload_struct = rekindle_types::governance::GovernanceSubkeyPayload {
        author_pseudonym: pseudo.clone(),
        entries: my_entries,
        signature: Vec::new(),
    };
    let signature = rekindle_secrets::derive::sign_with_pseudonym(
        &pseudonym_signing_key,
        &payload_struct.signing_bytes(),
    );
    payload_struct.signature = signature.to_vec();
    let payload = serde_json::to_vec(&payload_struct)
        .map_err(|e| format!("serialize governance entries: {e}"))?;
    let write_opts = veilid_core::SetDHTValueOptions {
        writer: Some(slot_kp),
        ..Default::default()
    };
    rc.set_dht_value(gov_key, my_slot, payload, Some(write_opts))
        .await
        .map_err(|e| format!("governance SMPL write failed: {e}"))?;

    let notification = CommunityEnvelope::Control(ControlPayload::GovernanceUpdated {
        governance_key: gov_key_str,
        subkey_index: my_slot,
        lamport_ts: entry.lamport(),
    });
    super::gossip::send_to_mesh(state, community_id, &notification)?;

    if let Some(mut current_state) = state_helpers::governance_state(state, community_id) {
        rekindle_governance::merge::apply_entry(&pseudo, &entry, &mut current_state);
        state_helpers::set_governance_state(state, community_id, current_state);
    }

    // Emit a UI snapshot event for any entry that invalidates a cached
    // store slice (roles list / channel tree). The snapshot is taken
    // after `set_governance_state` so the merged in-memory community
    // state already reflects the just-applied entry.
    let affects = classify_entry(&entry);
    if affects.roles {
        emit_community_event(
            state,
            CommunityEvent::RolesChanged {
                community_id: community_id.to_string(),
                roles: snapshot_roles(state, community_id),
            },
        );
    }
    if affects.channels_or_categories {
        let (channels, categories) = snapshot_channels_and_categories(state, community_id);
        emit_community_event(
            state,
            CommunityEvent::ChannelsUpdated {
                community_id: community_id.to_string(),
                channels,
                categories,
            },
        );
    }

    Ok(())
}
