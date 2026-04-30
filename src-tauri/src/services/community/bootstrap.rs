use std::sync::Arc;

use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::envelope::ControlPayload;
use rekindle_secrets::{derive, ed25519_dalek::SigningKey, mek::wrap_mek};
use serde_json::json;

use crate::state::AppState;

fn snapshot_governance_entries(governance: &GovernanceState) -> Vec<serde_json::Value> {
    let mut entries = Vec::new();

    if let Some(metadata) = &governance.metadata {
        entries.push(json!({
            "type": "community_meta",
            "name": metadata.name,
            "description": metadata.description,
            "icon_hash": metadata.icon_hash,
            "banner_hash": metadata.banner_hash,
            "lamport": metadata.lamport,
        }));
    }

    for (channel_id, channel) in &governance.channels {
        entries.push(json!({
            "type": "channel_created",
            "channel_id": hex::encode(channel_id.0),
            "name": channel.name,
            "channel_type": channel.channel_type,
            "record_key": channel.record_key,
            "category_id": channel.category_id.map(|id| hex::encode(id.0)),
            "position": channel.position,
            "lamport": channel.created_lamport,
        }));
    }

    for (role_id, role) in &governance.roles {
        entries.push(json!({
            "type": "role_definition",
            "role_id": hex::encode(role_id.0),
            "name": role.name,
            "permissions": role.permissions,
            "position": role.position,
            "color": role.color,
            "hoist": role.hoist,
            "mentionable": role.mentionable,
            "lamport": role.lamport,
        }));
    }

    for (member, role_ids) in &governance.role_assignments {
        for role_id in role_ids {
            entries.push(json!({
                "type": "role_assignment",
                "target": hex::encode(member.0),
                "role_id": hex::encode(role_id.0),
            }));
        }
    }

    for banned in &governance.bans {
        entries.push(json!({
            "type": "ban_entry",
            "target": hex::encode(banned.0),
        }));
    }

    for (member, timeout) in &governance.timeouts {
        entries.push(json!({
            "type": "timeout_entry",
            "target": hex::encode(member.0),
            "duration_seconds": timeout.duration_seconds,
            "started_at": timeout.started_at,
            "lamport": timeout.lamport,
        }));
    }

    for (category_id, category) in &governance.categories {
        entries.push(json!({
            "type": "category_created",
            "category_id": hex::encode(category_id.0),
            "name": category.name,
            "position": category.position,
            "lamport": category.created_lamport,
        }));
    }

    if let Some(onboarding) = &governance.onboarding {
        entries.push(json!({
            "type": "onboarding_config",
            "enabled": onboarding.enabled,
            "mode": onboarding.mode,
            "default_channels": onboarding.default_channels.iter().map(|id| hex::encode(id.0)).collect::<Vec<_>>(),
            "questions": onboarding.questions,
            "welcome_message": onboarding.welcome_message,
            "guide_steps": onboarding.guide_steps,
            "lamport": onboarding.lamport,
        }));
    }

    if let Some(welcome_screen) = &governance.welcome_screen {
        entries.push(json!({
            "type": "welcome_screen",
            "description": welcome_screen.description,
            "channels": welcome_screen.channels,
            "lamport": welcome_screen.lamport,
        }));
    }

    for (invite_id, invite) in &governance.invites {
        entries.push(json!({
            "type": "invite_created",
            "invite_id": hex::encode(invite_id),
            "code_hash": invite.code_hash,
            "max_uses": invite.max_uses,
            "expires_at": invite.expires_at,
            "encrypted_secrets": invite.encrypted_secrets,
            "lamport": invite.created_lamport,
        }));
    }

    entries
}

fn wrap_key_material(
    sender_signing_key: &SigningKey,
    joiner_pseudonym: &[u8; 32],
    key_material: &[u8],
) -> Result<Vec<u8>, String> {
    wrap_mek(sender_signing_key, joiner_pseudonym, key_material)
        .map_err(|e| format!("wrap bootstrap key material: {e}"))
}

pub fn build_bootstrap_response(
    state: &Arc<AppState>,
    community_id: &str,
    governance_key: &str,
    joiner_pseudonym_hex: &str,
) -> Result<Vec<u8>, String> {
    let joiner_pseudonym: [u8; 32] = hex::decode(joiner_pseudonym_hex)
        .map_err(|e| format!("invalid joiner pseudonym hex: {e}"))?
        .try_into()
        .map_err(|_| "joiner pseudonym must be 32 bytes")?;

    let identity_secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or("identity secret not available")?;
    let bootstrap_signing_key =
        derive::derive_community_pseudonym(&identity_secret, governance_key);

    let (governance_entries, member_list, wrapped_owner_keypair) = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or("community not found for bootstrap")?;
        let governance_state = community
            .governance_state
            .as_ref()
            .ok_or("governance state not cached")?;
        let governance_entries = snapshot_governance_entries(governance_state);
        let member_list = community
            .gossip
            .as_ref()
            .map(|gossip| {
                gossip
                    .online_members
                    .iter()
                    .map(|(pseudonym_key, member)| {
                        json!({
                            "pseudonym_key": pseudonym_key,
                            "status": member.status,
                            "route_blob": member.route_blob,
                            "last_seen": member.last_seen,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let wrapped_owner_keypair = community
            .dht_owner_keypair
            .as_ref()
            .map(|owner_keypair| {
                wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    owner_keypair.as_bytes(),
                )
            })
            .transpose()?
            .unwrap_or_default();

        (governance_entries, member_list, wrapped_owner_keypair)
    };

    let channel_meks = {
        let channels = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .map(|community| {
                    community
                        .channels
                        .iter()
                        .map(|channel| channel.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let channel_mek_cache = state.channel_mek_cache.lock();
        let mut entries = channel_mek_cache
            .iter()
            .filter(|((cid, _), _)| cid == community_id)
            .map(|((_, channel_id), mek)| {
                let wrapped = wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    &mek.to_wire_bytes(),
                )?;
                Ok(json!({
                    "channel_id": channel_id,
                    "mek_generation": mek.generation(),
                    "wrapped_mek": wrapped,
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        if entries.is_empty() {
            let community_mek_cache = state.mek_cache.lock();
            if let Some(mek) = community_mek_cache.get(community_id) {
                let wrapped = wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    &mek.to_wire_bytes(),
                )?;
                for channel_id in channels {
                    entries.push(json!({
                        "channel_id": channel_id,
                        "mek_generation": mek.generation(),
                        "wrapped_mek": wrapped.clone(),
                    }));
                }
            }
        }
        entries
    };

    let payload = ControlPayload::BootstrapResponse {
        governance_entries,
        member_list,
        channel_meks,
        recent_messages: Vec::new(),
        wrapped_owner_keypair,
    };
    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(payload);
    serde_json::to_vec(&envelope).map_err(|e| format!("serialize bootstrap response: {e}"))
}
