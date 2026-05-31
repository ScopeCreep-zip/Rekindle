//! Phase 23.C — invite-handler Tauri-runtime orchestration lifted from
//! `commands/community/invites.rs`. Hosts the three orchestrators
//! (`create_community_invite_inner`, `revoke_community_invite_inner`,
//! `list_community_invites_inner`).

use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::channels::community_channel::CommunityEvent;
use crate::commands::community::helpers::{
    hex_to_id_16, random_16_bytes, random_nonce, require_permission,
};
use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreatedDto {
    pub code: String,
    pub governance_key: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfoDto {
    pub code_hash: String,
    pub created_by: String,
    pub max_uses: Option<u32>,
    pub uses: u32,
    pub expires_at: Option<u64>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

pub async fn create_community_invite_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
) -> Result<InviteCreatedDto, String> {
    require_permission(state, &community_id, Permissions::CREATE_INSTANT_INVITE)?;

    let code = hex::encode(random_nonce(16));
    let code_hash = rekindle_secrets::invite::hash_invite_code(&code);

    let (governance_key, slot_seed, registry_key, community_name, inviter_route_blob) = {
        let communities = state.communities.read();
        let community = communities
            .get(&community_id)
            .ok_or("community not found")?;
        let governance_key = community
            .governance_key
            .clone()
            .or_else(|| Some(community.id.clone()))
            .ok_or("no governance key")?;
        let slot_seed = community
            .slot_seed
            .clone()
            .ok_or("no slot_seed available")?;
        let registry_key = community
            .member_registry_key
            .clone()
            .ok_or("no registry key for community")?;
        (
            governance_key,
            slot_seed,
            registry_key,
            community.name.clone(),
            state_helpers::our_route_blob(state).unwrap_or_default(),
        )
    };

    let mek_wire_b64 = {
        let cache = state.mek_cache.lock();
        let mek = cache.get(&community_id).ok_or("no MEK available")?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(mek.to_wire_bytes())
    };

    let channel_keys: Vec<rekindle_types::invite::ChannelKeyInfo> = {
        state_helpers::governance_state(state, &community_id)
            .map(|gov| {
                gov.channels
                    .iter()
                    .map(
                        |(channel_id, channel)| rekindle_types::invite::ChannelKeyInfo {
                            channel_id: hex::encode(channel_id.0),
                            record_key: channel.record_key.clone(),
                            name: channel.name.clone(),
                        },
                    )
                    .collect()
            })
            .unwrap_or_default()
    };

    let secrets = rekindle_types::invite::InviteSecrets {
        governance_key: governance_key.clone(),
        registry_key,
        inviter_route_blob,
        slot_seed,
        mek_wire_bytes: mek_wire_b64,
        channel_keys,
        community_name,
    };

    let secrets_json =
        serde_json::to_vec(&secrets).map_err(|e| format!("serialize invite secrets: {e}"))?;
    let encrypted = rekindle_secrets::invite::encrypt_invite_secrets(&code, &secrets_json)
        .map_err(|e| format!("encrypt invite secrets: {e}"))?;
    let encrypted_b64 = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&encrypted)
    };

    let expires_at = expires_in_seconds.map(|seconds| rekindle_utils::timestamp_secs() + seconds);
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::InviteCreated {
            invite_id: random_16_bytes(),
            code_hash: code_hash.clone(),
            max_uses: max_uses.unwrap_or(0),
            expires_at,
            encrypted_secrets: encrypted_b64,
            lamport,
        },
    )
    .await?;

    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let cid = community_id.clone();
    let raw_code = code.clone();
    let ch = code_hash.clone();
    let now = i64::try_from(rekindle_utils::timestamp_secs()).unwrap_or(0);
    let mu = max_uses.map_or(0, i64::from);
    let exp = expires_in_seconds.map(|seconds| now + i64::try_from(seconds).unwrap_or(0));
    let cid_for_db = cid.clone();
    crate::db_helpers::db_fire(pool, "persist invite locally", move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_invites (owner_key, community_id, code, code_hash, max_uses, expires_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![owner_key, cid_for_db, raw_code, ch, mu, exp, now],
        )?;
        Ok(())
    });

    if let Some(app) = state_helpers::app_handle(state) {
        let created_by = state_helpers::current_owner_key(state).unwrap_or_default();
        crate::event_dispatch::emit_live(
            &app,
            "community-event",
            &CommunityEvent::InviteCreated {
                community_id: cid.clone(),
                code_hash: code_hash.clone(),
                created_by,
                max_uses,
                uses: 0,
                expires_at,
                created_at: rekindle_utils::timestamp_secs(),
            },
        );
    }

    Ok(InviteCreatedDto {
        code,
        governance_key,
    })
}

pub async fn revoke_community_invite_inner(
    state: &SharedState,
    community_id: String,
    code_hash: String,
) -> Result<(), String> {
    require_permission(state, &community_id, Permissions::MANAGE_COMMUNITY)?;
    let lamport = state_helpers::increment_lamport(state, &community_id);
    crate::services::community::write_entry(
        state,
        &community_id,
        rekindle_types::governance::GovernanceEntry::InviteRevoked {
            invite_id: hex_to_id_16(&code_hash),
            lamport,
        },
    )
    .await?;

    if let Some(app) = state_helpers::app_handle(state) {
        crate::event_dispatch::emit_live(
            &app,
            "community-event",
            &CommunityEvent::InviteRevoked {
                community_id: community_id.clone(),
                code_hash: code_hash.clone(),
            },
        );
    }
    Ok(())
}

pub async fn list_community_invites_inner(
    pool: &DbPool,
    community_id: String,
) -> Result<Vec<InviteInfoDto>, String> {
    let cid = community_id.clone();
    let local_invites: Vec<(String, String, i64, Option<i64>, i64, i64)> =
        crate::db_helpers::db_call_or_default(pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT code_hash, code, max_uses, expires_at, created_at, uses \
                 FROM community_invites WHERE community_id = ?",
            )?;
            let rows = stmt.query_map([&cid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .await;

    Ok(local_invites
        .into_iter()
        .map(
            |(code_hash, code, max_uses, expires_at, created_at, uses)| InviteInfoDto {
                code_hash,
                created_by: String::new(),
                max_uses: if max_uses == 0 {
                    None
                } else {
                    Some(max_uses.try_into().unwrap_or(0))
                },
                uses: u32::try_from(uses).unwrap_or(0),
                expires_at: expires_at.map(|expires| expires.try_into().unwrap_or(0)),
                created_at: created_at.try_into().unwrap_or(0),
                code: Some(code),
            },
        )
        .collect())
}
