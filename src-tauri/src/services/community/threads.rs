use rekindle_protocol::dht::community::channel_record::{
    create_smpl_channel_record, read_all_channel_entries, read_all_channel_messages,
    write_member_message, ChannelMessage, ChannelRecordEntry,
};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::DHTManager;
use rekindle_records::schema::MAX_MEMBERS_PER_SEGMENT;

use crate::channels::community_channel::ThreadInfoDto;
use crate::commands::chat::Message;
use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

use super::threads_store::{load_thread_metadata, persist_thread_row};

pub async fn create_thread(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    name: &str,
    starter_message_id: &str,
    forum_tag: Option<String>,
    auto_archive_override: Option<u64>,
) -> Result<String, String> {
    let thread_id_bytes = rand::random::<[u8; 16]>();
    let thread_id = hex::encode(thread_id_bytes);
    let thread_type = parent_thread_type(state, community_id, channel_id)?;
    // Architecture §32 Phase 6 W19 line 4065 — caller may pick from
    // 1h / 24h / 3d / 7d. Reject anything outside the spec'd set so a
    // tampered client can't write 1-second auto-archive entries that
    // would race the merge.
    let auto_archive_seconds = match auto_archive_override {
        Some(secs) if [3600, 86_400, 259_200, 604_800].contains(&secs) => secs,
        Some(_) => return Err("auto_archive_seconds must be 3600, 86400, 259200, or 604800".into()),
        None => default_auto_archive_seconds(&thread_type),
    };
    let lamport = state_helpers::increment_lamport(state, community_id);
    let forum_tag_for_entry = forum_tag.clone();

    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::ThreadCreated {
            thread_id: rekindle_types::id::ThreadId(thread_id_bytes),
            parent_channel_id: rekindle_types::id::ChannelId(hex_to_id_16(channel_id)),
            name: name.to_string(),
            thread_type,
            record_key: None,
            invited: Vec::new(),
            forum_tag: forum_tag_for_entry,
            auto_archive_seconds,
            lamport,
        },
    )
    .await?;

    persist_thread_row(
        state,
        pool,
        community_id,
        &ThreadInfoDto {
            id: thread_id.clone(),
            channel_id: channel_id.to_string(),
            name: name.to_string(),
            starter_message_id: starter_message_id.to_string(),
            creator_pseudonym: my_pseudonym_hex(state, community_id)?,
            forum_tag: forum_tag.clone(),
            created_at: rekindle_utils::timestamp_secs(),
            archived: false,
            auto_archive_seconds: u32::try_from(auto_archive_seconds).unwrap_or(u32::MAX),
            last_message_at: 0,
            message_count: 0,
        },
    )
    .await?;

    Ok(thread_id)
}

pub async fn list_threads(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoDto>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let gov_state =
        state_helpers::governance_state(state, community_id).ok_or("governance state not loaded")?;
    let mut threads = Vec::new();

    for (thread_id, thread) in gov_state
        .threads
        .iter()
        .filter(|(_, thread)| hex::encode(thread.parent_channel_id.0) == channel_id)
    {
        let mut dto = load_thread_metadata(pool, &owner_key, community_id, &hex::encode(thread_id.0))
            .await?
            .unwrap_or_else(|| ThreadInfoDto {
                id: hex::encode(thread_id.0),
                channel_id: channel_id.to_string(),
                name: thread.name.clone(),
                starter_message_id: String::new(),
                creator_pseudonym: hex::encode(thread.creator.0),
                forum_tag: thread.forum_tag.clone(),
                created_at: thread.created_lamport,
                archived: false,
                auto_archive_seconds: u32::try_from(thread.auto_archive_seconds)
                    .unwrap_or(u32::MAX),
                last_message_at: 0,
                message_count: 0,
            });
        dto.name = thread.name.clone();
        dto.creator_pseudonym = hex::encode(thread.creator.0);
        dto.forum_tag = thread.forum_tag.clone();
        dto.auto_archive_seconds =
            u32::try_from(thread.auto_archive_seconds).unwrap_or(u32::MAX);

        let (last_lamport, last_activity, message_count) =
            thread_activity(state, community_id, thread.record_key.as_deref()).await?;
        dto.last_message_at = last_activity;
        dto.message_count = message_count;
        dto.archived = is_thread_archived(thread, last_lamport, last_activity);
        threads.push(dto);
    }

    threads.sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at).then_with(|| a.name.cmp(&b.name)));
    Ok(threads)
}

pub async fn list_active_threads(
    state: &SharedState,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
) -> Result<Vec<ThreadInfoDto>, String> {
    let threads = list_threads(state, pool, community_id, channel_id).await?;
    Ok(filter_active_threads(threads))
}

pub async fn send_thread_message(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(), String> {
    let (record_key, channel_message) = ensure_thread_record_and_message(state, community_id, thread_id, body).await?;
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let writer = thread_writer(state, community_id)?;
    let mgr = DHTManager::new(rc);

    let (author_pseudo, signing_key) =
        state_helpers::pseudonym_credentials(state, community_id)?;
    write_member_message(
        &mgr,
        &record_key,
        my_slot_index(state, community_id)?,
        writer,
        author_pseudo,
        &signing_key,
        &channel_message,
    )
    .await
    .map_err(|e| format!("thread SMPL write failed: {e}"))?;

    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::ThreadMessageReceived {
            thread_id: thread_id.to_string(),
            message_id: channel_message.message_id.clone().unwrap_or_default(),
            sender_pseudonym: channel_message.sender_pseudonym.clone(),
            ciphertext: channel_message.ciphertext.clone(),
            mek_generation: channel_message.mek_generation,
            timestamp: channel_message.timestamp / 1000,
            reply_to_id: None,
        }),
    )?;

    Ok(())
}

pub async fn load_thread_messages(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    limit: u32,
    before_timestamp: Option<u64>,
) -> Result<Vec<Message>, String> {
    let thread = thread_state(state, community_id, thread_id)?;
    let Some(record_key) = thread.record_key.as_deref() else {
        return Ok(Vec::new());
    };
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    // Architecture §8 line 1626 — keep subkey_index alongside each
    // ChannelMessage so the AAD can be reconstructed for decrypt.
    let entries = read_all_channel_entries(&rc, record_key, thread_member_count())
        .await
        .map_err(|e| format!("read thread messages failed: {e}"))?;
    let mut items: Vec<(u32, ChannelMessage)> = entries
        .into_iter()
        .filter_map(|item| match item.entry {
            ChannelRecordEntry::Message(msg) => Some((item.subkey_index, msg)),
            _ => None,
        })
        .collect();
    items.sort_by(|a, b| {
        a.1.lamport_ts
            .cmp(&b.1.lamport_ts)
            .then_with(|| a.1.sender_pseudonym.cmp(&b.1.sender_pseudonym))
    });
    let before_ms = before_timestamp.map_or(u64::MAX, |ts| ts.saturating_mul(1000));
    let my_pseudonym = my_pseudonym_hex(state, community_id)?;
    let mut messages: Vec<Message> = items
        .into_iter()
        .filter(|(_, message)| message.timestamp < before_ms)
        .rev()
        .take(limit.min(200) as usize)
        .map(|(subkey_index, message)| Message {
            id: 0,
            sender_id: message.sender_pseudonym.clone(),
            body: decrypt_thread_body(
                state,
                community_id,
                record_key,
                subkey_index,
                message.lamport_ts,
                &message.ciphertext,
                message.mek_generation,
            ),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: i64::try_from(message.timestamp).unwrap_or(i64::MAX),
            is_own: message.sender_pseudonym == my_pseudonym,
            server_message_id: message.message_id.clone(),
            reactions: None,
            pinned: None,
            poll: None,
            forwarded_from_author: None,
            attachment: None,
            flags: 0,
        })
        .collect();
    messages.reverse();
    Ok(messages)
}

pub async fn archive_thread(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::ThreadArchived {
            thread_id: rekindle_types::id::ThreadId(hex_to_id_16(thread_id)),
            lamport,
        },
    )
    .await
}

fn is_thread_archived(
    thread: &rekindle_governance::state::ThreadState,
    last_lamport: u64,
    last_activity_secs: u64,
) -> bool {
    let manually_archived = thread
        .archived_lamport
        .is_some_and(|archived| last_lamport <= archived);
    let auto_archived = last_activity_secs > 0
        && last_activity_secs.saturating_add(thread.auto_archive_seconds)
            < rekindle_utils::timestamp_secs();
    manually_archived || auto_archived
}

async fn thread_activity(
    state: &SharedState,
    _community_id: &str,
    record_key: Option<&str>,
) -> Result<(u64, u64, u32), String> {
    let Some(record_key) = record_key else {
        return Ok((0, 0, 0));
    };
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let messages = read_all_channel_messages(&rc, record_key, thread_member_count())
        .await
        .map_err(|e| format!("read thread activity failed: {e}"))?;
    let last_lamport = messages.iter().map(|message| message.lamport_ts).max().unwrap_or(0);
    let last_activity = messages.iter().map(|message| message.timestamp / 1000).max().unwrap_or(0);
    Ok((last_lamport, last_activity, u32::try_from(messages.len()).unwrap_or(u32::MAX)))
}

async fn ensure_thread_record_and_message(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    body: &str,
) -> Result<(String, ChannelMessage), String> {
    let thread = thread_state(state, community_id, thread_id)?;
    let record_key = match thread.record_key.clone() {
        Some(record_key) => record_key,
        None => create_lazy_thread_record(state, community_id, thread_id, &thread).await?,
    };
    let slot_index = my_slot_index(state, community_id)?;
    let message = build_thread_message(state, community_id, &record_key, slot_index, body)?;
    Ok((record_key, message))
}

async fn create_lazy_thread_record(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
    thread: &rekindle_governance::state::ThreadState,
) -> Result<String, String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let slot_seed = slot_seed_bytes(state, community_id)?;
    let mgr = DHTManager::new(rc);
    let (record_key, _) = create_smpl_channel_record(&mgr, &slot_seed)
        .await
        .map_err(|e| format!("create lazy thread record failed: {e}"))?;

    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            community.open_community_records.channel_keys.push(record_key.clone());
        }
    }
    state_helpers::track_open_records(state, std::slice::from_ref(&record_key));
    let _ = crate::services::community::watch_community_records(state, community_id).await;

    let lamport = state_helpers::increment_lamport(state, community_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::ThreadCreated {
            thread_id: rekindle_types::id::ThreadId(hex_to_id_16(thread_id)),
            parent_channel_id: thread.parent_channel_id,
            name: thread.name.clone(),
            thread_type: thread.thread_type.clone(),
            record_key: Some(record_key.clone()),
            invited: thread.invited.clone(),
            forum_tag: thread.forum_tag.clone(),
            auto_archive_seconds: thread.auto_archive_seconds,
            lamport,
        },
    )
    .await?;

    Ok(record_key)
}

fn build_thread_message(
    state: &SharedState,
    community_id: &str,
    record_key: &str,
    slot_index: u32,
    body: &str,
) -> Result<ChannelMessage, String> {
    // Architecture §8 line 1626 — bind ciphertext to (thread_record_key,
    // subkey_index, lamport_ts) so a thread message can't be replayed
    // into a different thread or reordered.
    let lamport_ts = state_helpers::increment_lamport(state, community_id);
    let aad = rekindle_crypto::group::media_key::ChannelAad {
        channel_record_key: record_key.as_bytes(),
        subkey_index: slot_index,
        lamport_ts,
    };
    let ciphertext = {
        let mek_cache = state.mek_cache.lock();
        let mek = mek_cache.get(community_id).ok_or("MEK not available")?;
        mek.encrypt_with_aad(body.as_bytes(), aad)
            .map_err(|e| format!("MEK encryption failed: {e}"))?
    };
    // Architecture §28.5 — thread reply mention metadata. Same parse +
    // permission gate as channel messages. Reuses the centralized
    // resolver in `channel_messages` so mention validation stays
    // consistent across send paths.
    let sender_hex = my_pseudonym_hex(state, community_id)?;
    let (mentioned_pseudonyms, mentioned_roles, mention_flags) =
        crate::services::community::channel_messages::resolve_outbound_mentions(
            state,
            community_id,
            &sender_hex,
            body,
        );
    Ok(ChannelMessage {
        sequence: next_thread_sequence(state, community_id),
        sender_pseudonym: sender_hex,
        ciphertext,
        mek_generation: current_mek_generation(state, community_id)?,
        timestamp: crate::db::timestamp_now().try_into().unwrap_or_default(),
        reply_to: None,
        lamport_ts,
        message_id: Some(format!("tmsg_{}", uuid::Uuid::new_v4().simple())),
        attachment: None,
        flags: mention_flags,
        mentioned_pseudonyms,
        mentioned_roles,
    })
}

fn decrypt_thread_body(
    state: &SharedState,
    community_id: &str,
    record_key: &str,
    subkey_index: u32,
    lamport_ts: u64,
    ciphertext: &[u8],
    mek_generation: u64,
) -> String {
    let mek_cache = state.mek_cache.lock();
    let Some(mek) = mek_cache.get(community_id) else {
        return String::new();
    };
    if mek.generation() != mek_generation {
        return String::new();
    }
    let aad = rekindle_crypto::group::media_key::ChannelAad {
        channel_record_key: record_key.as_bytes(),
        subkey_index,
        lamport_ts,
    };
    if let Ok(bytes) = mek.decrypt_with_aad(ciphertext, aad) {
        if let Ok(text) = String::from_utf8(bytes) {
            return text;
        }
    }
    // Architecture §8 fallback for legacy thread messages written
    // before AAD landed.
    mek.decrypt(ciphertext)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

fn thread_state(
    state: &SharedState,
    community_id: &str,
    thread_id: &str,
) -> Result<rekindle_governance::state::ThreadState, String> {
    let gov_state =
        state_helpers::governance_state(state, community_id).ok_or("governance state not loaded")?;
    gov_state
        .threads
        .get(&rekindle_types::id::ThreadId(hex_to_id_16(thread_id)))
        .cloned()
        .ok_or("thread not found".into())
}

fn parent_thread_type(state: &SharedState, community_id: &str, channel_id: &str) -> Result<String, String> {
    let gov_state =
        state_helpers::governance_state(state, community_id).ok_or("governance state not loaded")?;
    Ok(gov_state
        .channels
        .get(&rekindle_types::id::ChannelId(hex_to_id_16(channel_id)))
        .map(|channel| {
            if channel.channel_type == "forum" {
                "forum_post".to_string()
            } else {
                "public".to_string()
            }
        })
        .ok_or("parent channel not found")?)
}

fn default_auto_archive_seconds(thread_type: &str) -> u64 {
    match thread_type {
        "announcement" => 72 * 60 * 60,
        "forum_post" => 7 * 24 * 60 * 60,
        _ => 24 * 60 * 60,
    }
}

fn my_pseudonym_hex(state: &SharedState, community_id: &str) -> Result<String, String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.my_pseudonym_key.clone())
        .ok_or("no pseudonym key for community".into())
}

fn my_slot_index(state: &SharedState, community_id: &str) -> Result<u32, String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.my_subkey_index)
        .ok_or("community subkey index missing".into())
}

fn thread_writer(state: &SharedState, community_id: &str) -> Result<veilid_core::KeyPair, String> {
    state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.slot_keypair.clone())
        .ok_or("slot keypair missing")?
        .parse()
        .map_err(|e| format!("invalid slot keypair: {e}"))
}

fn current_mek_generation(state: &SharedState, community_id: &str) -> Result<u64, String> {
    state
        .communities
        .read()
        .get(community_id)
        .map(|community| community.mek_generation)
        .ok_or("community not found".into())
}

fn next_thread_sequence(state: &SharedState, community_id: &str) -> u64 {
    let mut communities = state.communities.write();
    let Some(community) = communities.get_mut(community_id) else {
        return 1;
    };
    let sequence = community.channel_sequences.entry("__thread__".into()).or_insert(0);
    *sequence += 1;
    *sequence
}

fn slot_seed_bytes(state: &SharedState, community_id: &str) -> Result<[u8; 32], String> {
    let slot_seed = state
        .communities
        .read()
        .get(community_id)
        .and_then(|community| community.slot_seed.clone())
        .ok_or("no slot seed available for community")?;
    hex::decode(slot_seed)
        .map_err(|e| format!("invalid slot seed hex: {e}"))?
        .try_into()
        .map_err(|_| "slot seed must be 32 bytes".into())
}

fn thread_member_count() -> u32 {
    u32::try_from(MAX_MEMBERS_PER_SEGMENT).unwrap_or(u32::MAX)
}

fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0u8; 16])
}

fn filter_active_threads(threads: Vec<ThreadInfoDto>) -> Vec<ThreadInfoDto> {
    threads
        .into_iter()
        .filter(|thread| !thread.archived)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{filter_active_threads, is_thread_archived};
    use crate::channels::community_channel::ThreadInfoDto;

    #[test]
    fn thread_reactivates_when_message_lamport_exceeds_archive_lamport() {
        let thread = rekindle_governance::state::ThreadState {
            parent_channel_id: rekindle_types::id::ChannelId([1; 16]),
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            creator: rekindle_types::id::PseudonymKey([2; 32]),
            created_lamport: 3,
            archived_lamport: Some(8),
        };
        let recent_activity = rekindle_utils::timestamp_secs();

        assert!(is_thread_archived(&thread, 8, recent_activity));
        assert!(!is_thread_archived(&thread, 9, recent_activity));
    }

    #[test]
    fn filter_active_threads_excludes_archived_entries() {
        let active = ThreadInfoDto {
            id: "active".into(),
            channel_id: "channel".into(),
            name: "active".into(),
            starter_message_id: "m1".into(),
            creator_pseudonym: "creator".into(),
            forum_tag: None,
            created_at: 1,
            archived: false,
            auto_archive_seconds: 86_400,
            last_message_at: 2,
            message_count: 3,
        };
        let archived = ThreadInfoDto {
            id: "archived".into(),
            archived: true,
            ..active.clone()
        };

        let filtered = filter_active_threads(vec![active.clone(), archived]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, active.id);
    }
}
