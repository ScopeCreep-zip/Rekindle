use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::{
    read_all_channel_entries, ChannelHandRaise, ChannelRecordEntry,
};
use rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT;
use tauri::Manager;

use crate::state::SharedState;
use crate::state_helpers;

struct StageWriteContext {
    community_id: String,
    channel_key: String,
    slot_index: u32,
    slot_keypair: veilid_core::KeyPair,
}

pub async fn persist_hand_raise(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
    raised: bool,
) -> Result<(), String> {
    let context = stage_write_context(state, community_id, channel_id)?;
    let hand_raise = ChannelHandRaise {
        raised,
        lamport: state_helpers::increment_lamport(state, community_id),
    };
    write_hand_raise_once(state, &context, &hand_raise).await
}

pub async fn list_hand_raises(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<String>, String> {
    let context = stage_write_context(state, community_id, channel_id)?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let entries = read_all_channel_entries(
        &rc,
        &context.channel_key,
        u32::try_from(MAX_MEMBERS_PER_SEGMENT).unwrap_or(u32::MAX),
    )
    .await
    .map_err(|e| format!("read stage hand raises: {e}"))?;

    let pseudonyms_by_subkey = stage_pseudonyms_by_subkey(state, community_id).await?;
    let mut latest_by_subkey = std::collections::HashMap::<u32, (u64, bool)>::new();
    for item in entries {
        let ChannelRecordEntry::HandRaise(hand_raise) = item.entry else {
            continue;
        };
        let replace = latest_by_subkey
            .get(&item.subkey_index)
            .is_none_or(|(lamport, _)| hand_raise.lamport >= *lamport);
        if replace {
            latest_by_subkey.insert(item.subkey_index, (hand_raise.lamport, hand_raise.raised));
        }
    }

    let mut raised = latest_by_subkey
        .into_iter()
        .filter_map(|(subkey_index, (_, is_raised))| {
            is_raised
                .then(|| pseudonyms_by_subkey.get(&subkey_index).cloned())
                .flatten()
        })
        .collect::<Vec<_>>();
    raised.sort();
    Ok(raised)
}

fn stage_write_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<StageWriteContext, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found for stage write")?;
    let slot_keypair = community
        .slot_keypair
        .clone()
        .ok_or("slot keypair missing for stage write")?
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    Ok(StageWriteContext {
        community_id: community_id.to_string(),
        channel_key: community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or("channel record key missing for stage write")?,
        slot_index: community
            .my_subkey_index
            .ok_or("subkey index missing for stage write")?,
        slot_keypair,
    })
}

async fn write_hand_raise_once(
    state: &SharedState,
    context: &StageWriteContext,
    hand_raise: &ChannelHandRaise,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, &context.community_id)?;
    rekindle_protocol::dht::community::channel_record::write_member_hand_raise(
        &mgr,
        &context.channel_key,
        context.slot_index,
        context.slot_keypair.clone(),
        author_pseudo,
        &signing_key,
        hand_raise,
    )
    .await
    .map_err(|e| format!("SMPL hand raise write failed: {e}"))
}

async fn stage_pseudonyms_by_subkey(
    state: &SharedState,
    community_id: &str,
) -> Result<std::collections::HashMap<u32, String>, String> {
    let community = state
        .communities
        .read()
        .get(community_id)
        .cloned()
        .ok_or("community not found for stage reads")?;

    let mut pseudonyms = std::collections::HashMap::new();
    if let (Some(my_subkey_index), Some(my_pseudonym)) =
        (community.my_subkey_index, community.my_pseudonym_key)
    {
        pseudonyms.insert(my_subkey_index, my_pseudonym);
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let app_handle = state.app_handle.read().clone().ok_or("app handle missing")?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or("database pool missing")?;
    let community_id = community_id.to_string();
    let db_subkeys = crate::db_helpers::db_call(&pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, subkey_index FROM community_members \
             WHERE owner_key = ?1 AND community_id = ?2 AND subkey_index IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, community_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                u32::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await?;
    for (pseudonym_key, subkey_index) in db_subkeys {
        pseudonyms.insert(subkey_index, pseudonym_key);
    }

    Ok(pseudonyms)
}
