use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::state::SharedState;
use crate::state_helpers;

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
            .map(|payload| payload.entries)
            .unwrap_or_default(),
            _ => Vec::new(),
        };
    my_entries.push(entry.clone());

    let payload = serde_json::to_vec(&rekindle_types::governance::GovernanceSubkeyPayload {
        author_pseudonym: pseudo.clone(),
        entries: my_entries,
    })
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

    Ok(())
}
