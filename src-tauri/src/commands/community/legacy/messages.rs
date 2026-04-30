use std::collections::{BTreeMap, HashMap, HashSet};

use crate::commands::chat::{Message, ReactionGroup};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

use super::channel_materialize::{
    build_poll_states, build_reaction_groups, decrypt_channel_record_message,
};

pub(crate) fn merge_message_lists(existing: &mut Vec<Message>, fallback: Vec<Message>, limit: u32) {
    let mut seen = HashMap::new();
    for (idx, message) in existing.iter().enumerate() {
        seen.insert(message_identity(message), idx);
    }
    for message in fallback {
        let key = message_identity(&message);
        if let Some(existing_idx) = seen.get(&key).copied() {
            merge_message_fields(&mut existing[existing_idx], &message);
        } else {
            seen.insert(key, existing.len());
            existing.push(message);
        }
    }
    existing.sort_by_key(|message| message.timestamp);
    let max_len = usize::try_from(limit.max(1)).unwrap_or(100);
    if existing.len() > max_len {
        let start = existing.len() - max_len;
        existing.drain(0..start);
    }
}

fn merge_message_fields(existing: &mut Message, incoming: &Message) {
    if existing.server_message_id.is_none() {
        existing
            .server_message_id
            .clone_from(&incoming.server_message_id);
    }
    existing.decryption_failed |= incoming.decryption_failed;
    existing.reactions =
        merge_reaction_groups(existing.reactions.take(), incoming.reactions.clone());
    if incoming.pinned == Some(true) {
        existing.pinned = Some(true);
    }
    if incoming.poll.is_some() {
        existing.poll.clone_from(&incoming.poll);
    }
}

fn merge_reaction_groups(
    existing: Option<Vec<ReactionGroup>>,
    incoming: Option<Vec<ReactionGroup>>,
) -> Option<Vec<ReactionGroup>> {
    let mut groups: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for reaction in existing
        .into_iter()
        .flatten()
        .chain(incoming.into_iter().flatten())
    {
        groups
            .entry(reaction.emoji)
            .or_default()
            .extend(reaction.reactors);
    }
    if groups.is_empty() {
        None
    } else {
        Some(
            groups
                .into_iter()
                .map(|(emoji, reactors): (String, HashSet<String>)| {
                    let mut reactors: Vec<String> = reactors.into_iter().collect();
                    reactors.sort();
                    ReactionGroup {
                        count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
                        emoji,
                        reactors,
                    }
                })
                .collect(),
        )
    }
}

fn message_identity(message: &Message) -> String {
    match &message.server_message_id {
        Some(message_id) => format!("id:{message_id}"),
        None => format!(
            "ts:{}:{}:{}",
            message.timestamp, message.sender_id, message.body
        ),
    }
}

pub(crate) async fn clear_registry_presence_slot(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    pseudonym_key: &str,
) -> Result<(), String> {
    let (registry_key, slot_seed_hex, my_pseudonym, my_subkey_index) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community
                .member_registry_key
                .clone()
                .ok_or("no member registry key")?,
            community
                .slot_seed
                .clone()
                .ok_or("no slot seed available")?,
            community.my_pseudonym_key.clone(),
            community.my_subkey_index,
        )
    };

    let subkey_index = if my_pseudonym.as_deref() == Some(pseudonym_key) {
        my_subkey_index.ok_or("no local subkey index")?
    } else {
        let owner_key = state_helpers::current_owner_key(state)?;
        let cid = community_id.to_string();
        let pk = pseudonym_key.to_string();
        db_call(pool, move |conn| {
            conn.query_row(
                "SELECT subkey_index FROM community_members \
                 WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                rusqlite::params![owner_key, cid, pk],
                |row| row.get::<_, i64>(0),
            )
            .map(|idx| u32::try_from(idx).unwrap_or(0))
        })
        .await?
    };

    let slot_seed_bytes: [u8; 32] = hex::decode(&slot_seed_hex)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes")?;
    let slot_keypair =
        rekindle_secrets::derive::derive_slot_keypair(&slot_seed_bytes, subkey_index)
            .map_err(|e| format!("slot keypair derivation failed: {e}"))?;
    let writer = crate::services::community::create::slot_signing_to_veilid(&slot_keypair);
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let record_key = registry_key
        .parse::<veilid_core::RecordKey>()
        .map_err(|e| format!("invalid registry key: {e}"))?;
    let write_opts = veilid_core::SetDHTValueOptions {
        writer: Some(writer),
        ..Default::default()
    };
    rc.set_dht_value(record_key, subkey_index, Vec::new(), Some(write_opts))
        .await
        .map_err(|e| format!("registry slot clear failed: {e}"))?;
    Ok(())
}

pub(crate) async fn load_channel_messages_from_smpl(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    before_timestamp: Option<u64>,
    limit: u32,
) -> Result<Vec<Message>, String> {
    use rekindle_protocol::dht::community::channel_record::{
        read_all_channel_entries, ChannelRecordEntry,
    };

    let (channel_key, my_pseudonym) = {
        let communities = state.communities.read();
        let community = communities.get(community_id).ok_or("community not found")?;
        (
            community.channel_log_keys.get(channel_id).cloned(),
            community.my_pseudonym_key.clone().unwrap_or_default(),
        )
    };
    let Some(channel_key) = channel_key else {
        return Ok(Vec::new());
    };
    let Some(rc) = state_helpers::routing_context(state) else {
        return Ok(Vec::new());
    };

    let channel_entries = read_all_channel_entries(&rc, &channel_key, 255)
        .await
        .map_err(|e| format!("read SMPL channel history: {e}"))?;

    let subkey_pseudonyms = load_channel_subkey_pseudonyms(state, pool, community_id)
        .await
        .unwrap_or_default();
    let reaction_groups = build_reaction_groups(&channel_entries, &subkey_pseudonyms);
    let poll_states = build_poll_states(&channel_entries, &subkey_pseudonyms, &my_pseudonym);
    let mut filtered: Vec<rekindle_protocol::dht::community::channel_record::ChannelMessage> =
        channel_entries
            .iter()
            .filter_map(|item| match &item.entry {
                ChannelRecordEntry::Message(message)
                    if before_timestamp.is_none_or(|before| message.timestamp < before) =>
                {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect();
    if filtered.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
        let start = filtered.len() - usize::try_from(limit).unwrap_or(filtered.len());
        filtered = filtered.split_off(start);
    }

    if filtered.is_empty() {
        return Ok(Vec::new());
    }

    let hydrated_messages: Vec<Message> = filtered
        .iter()
        .map(|message| {
            let decrypted = decrypt_channel_record_message(
                state,
                community_id,
                channel_id,
                message.mek_generation,
                &message.ciphertext,
            );
            Message {
                id: 0,
                is_own: message.sender_pseudonym == my_pseudonym,
                sender_id: message.sender_pseudonym.clone(),
                body: decrypted.body,
                decryption_failed: decrypted.decryption_failed,
                automod_blurred: false,
                timestamp: i64::try_from(message.timestamp).unwrap_or(i64::MAX),
                server_message_id: message.message_id.clone(),
                reactions: message
                    .message_id
                    .as_ref()
                    .and_then(|message_id| reaction_groups.get(message_id).cloned()),
                pinned: None,
                poll: message
                    .message_id
                    .as_ref()
                    .and_then(|message_id| poll_states.get(message_id).cloned()),
            }
        })
        .collect();

    if let Ok(owner_key) = state_helpers::current_owner_key(state) {
        let chid = channel_id.to_string();
        let messages_for_db = filtered.clone();
        let owner_key_for_db = owner_key.clone();
        let state_for_db = state.clone();
        let community_id_for_db = community_id.to_string();
        let channel_id_for_db = channel_id.to_string();
        let _ = db_call(pool, move |conn| {
            for message in &messages_for_db {
                let Some(message_id) = message.message_id.as_deref() else {
                    continue;
                };
                let decrypted = decrypt_channel_record_message(
                    &state_for_db,
                    &community_id_for_db,
                    &channel_id_for_db,
                    message.mek_generation,
                    &message.ciphertext,
                );
                let _ = crate::message_repo::insert_channel_message_with_protocol_metadata(
                    conn,
                    &owner_key_for_db,
                    &chid,
                    &message.sender_pseudonym,
                    &decrypted.body,
                    i64::try_from(message.timestamp).unwrap_or(i64::MAX),
                    true,
                    Some(i64::try_from(message.mek_generation).unwrap_or(i64::MAX)),
                    message_id,
                    message.lamport_ts,
                    false,
                );
            }
            Ok(())
        })
        .await;
    }

    Ok(hydrated_messages)
}

async fn load_channel_subkey_pseudonyms(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
) -> Result<HashMap<u32, String>, String> {
    let mut subkeys = {
        let communities = state.communities.read();
        let mut subkeys = HashMap::new();
        if let Some(community) = communities.get(community_id) {
            if let (Some(my_subkey_index), Some(my_pseudonym_key)) = (
                community.my_subkey_index,
                community.my_pseudonym_key.clone(),
            ) {
                subkeys.insert(my_subkey_index, my_pseudonym_key);
            }
        }
        subkeys
    };
    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id = community_id.to_string();
    let db_subkeys = db_call(pool, move |conn| {
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
        subkeys.insert(subkey_index, pseudonym_key);
    }
    Ok(subkeys)
}
