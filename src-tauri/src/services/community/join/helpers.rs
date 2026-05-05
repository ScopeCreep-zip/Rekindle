use std::sync::Arc;

use rekindle_secrets::derive;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::state::AppState;

pub(crate) fn role_id_to_legacy_u32(role_id: &rekindle_types::id::RoleId) -> u32 {
    u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]])
}

pub(super) fn find_invite_in_governance(
    subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)],
    code_hash: &str,
) -> Result<String, String> {
    let mut revoked_ids: std::collections::HashSet<[u8; 16]> = std::collections::HashSet::new();
    for (_, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteRevoked { invite_id, .. } = entry {
                revoked_ids.insert(*invite_id);
            }
        }
    }

    for (_, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteCreated {
                invite_id,
                code_hash: ref ch,
                encrypted_secrets,
                expires_at,
                lamport: _,
                ..
            } = entry
            {
                if ch == code_hash {
                    if revoked_ids.contains(invite_id) {
                        return Err("invite has been revoked".into());
                    }
                    if let Some(exp) = expires_at {
                        if rekindle_utils::timestamp_secs() > *exp {
                            return Err("invite has expired".into());
                        }
                    }
                    return Ok(encrypted_secrets.clone());
                }
            }
        }
    }
    Err("invalid invite code — no matching invite found in governance".into())
}

pub(super) async fn open_channel_records(
    rc: &veilid_core::RoutingContext,
    state: &Arc<AppState>,
    community_id: &str,
) {
    let channel_keys: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|cs| cs.channel_log_keys.values().cloned().collect())
            .unwrap_or_default()
    };

    for key_str in &channel_keys {
        if let Ok(typed_key) = key_str.parse::<veilid_core::RecordKey>() {
            if let Err(e) = rc.open_dht_record(typed_key, None).await {
                tracing::debug!(key = %key_str, error = %e, "failed to open channel record on join");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

pub(super) fn spawn_join_announcements(
    state: Arc<AppState>,
    community_id: String,
    pseudonym_key: String,
    subkey_index: u32,
) {
    let display_name = crate::state_helpers::identity_display_name(&state);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let our_route = crate::state_helpers::our_route_blob(&state);
        let status = match crate::state_helpers::identity_status(&state)
            .unwrap_or(crate::state::UserStatus::Online)
        {
            crate::state::UserStatus::Online => "online",
            crate::state::UserStatus::Away => "away",
            crate::state::UserStatus::Busy => "busy",
            crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => "offline",
        };

        let joined_envelope =
            rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
                rekindle_protocol::dht::community::envelope::ControlPayload::MemberJoined {
                    pseudonym_key: pseudonym_key.clone(),
                    display_name,
                    role_ids: vec![0],
                    status: status.to_string(),
                    route_blob: our_route,
                },
            );
        let _ = crate::services::community::send_to_mesh(&state, &community_id, &joined_envelope);

        tracing::info!(
            community = %community_id,
            slot = subkey_index,
            "broadcasted MemberJoined via gossip"
        );
    });
}

pub(super) fn default_community_name(governance_key: &str) -> String {
    format!(
        "Community {}",
        &governance_key[..8.min(governance_key.len())]
    )
}

pub(crate) fn try_derive_slot_keypair(
    state: &Arc<AppState>,
    community_id: &str,
    seed_hex: &str,
    subkey_idx: u32,
) -> Option<String> {
    let seed_bytes = hex::decode(seed_hex).ok()?;
    let seed_array: [u8; 32] = seed_bytes.as_slice().try_into().ok()?;
    match derive::derive_slot_keypair(&seed_array, subkey_idx) {
        Ok(sk) => {
            let kp = super::super::create::slot_signing_to_veilid(&sk);
            let kp_str = kp.to_string();
            {
                let mut communities = state.communities.write();
                if let Some(c) = communities.get_mut(community_id) {
                    c.slot_keypair = Some(kp_str.clone());
                }
            }
            Some(kp_str)
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to derive slot keypair from seed");
            None
        }
    }
}
