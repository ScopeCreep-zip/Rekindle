use std::sync::Arc;

use rekindle_protocol::dht::community::channel_record::{
    read_all_channel_entries, write_member_poll_close, write_member_poll_create,
    write_member_poll_vote, ChannelPollClose, ChannelPollCreate, ChannelPollVote,
    ChannelRecordEntry,
};

use crate::state::SharedState;
use crate::state_helpers;

struct PollWriteContext {
    channel_key: String,
    slot_index: u32,
    slot_keypair: veilid_core::KeyPair,
}

pub async fn persist_poll_create(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    question: &str,
    answers: Vec<String>,
    multi_select: bool,
    expires_at: Option<u64>,
) -> Result<String, String> {
    validate_poll_create(question, &answers)?;
    let context = poll_write_context(state, community_id, channel_id)?;
    let poll_id = random_poll_id();
    let entry = ChannelPollCreate {
        poll_id,
        message_id: message_id.to_string(),
        question: question.trim().to_string(),
        answers: answers
            .into_iter()
            .map(|answer| answer.trim().to_string())
            .collect(),
        multi_select,
        expires_at,
        lamport: state_helpers::increment_lamport(state, community_id),
    };
    write_poll_create_once(state, &context, &entry).await?;
    Ok(hex::encode(poll_id))
}

pub async fn persist_poll_vote(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    selected_answers: Vec<u8>,
) -> Result<(), String> {
    if selected_answers.is_empty() {
        return Err("at least one answer must be selected".to_string());
    }
    let context = poll_write_context(state, community_id, channel_id)?;
    let entry = ChannelPollVote {
        poll_id: parse_poll_id(poll_id_hex)?,
        selected_answers: dedupe_selected_answers(selected_answers),
        lamport: state_helpers::increment_lamport(state, community_id),
    };
    write_poll_vote_once(state, &context, &entry).await
}

pub async fn persist_poll_close(
    state: &Arc<crate::state::AppState>,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    allow_moderator_override: bool,
) -> Result<(), String> {
    let context = poll_write_context(state, community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    if !allow_moderator_override {
        ensure_poll_author(state, &context, poll_id).await?;
    }
    let entry = ChannelPollClose {
        poll_id,
        lamport: state_helpers::increment_lamport(state, community_id),
    };
    write_poll_close_once(state, &context, &entry).await
}

fn poll_write_context(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
) -> Result<PollWriteContext, String> {
    let communities = state.communities.read();
    let community = communities
        .get(community_id)
        .ok_or("community not found for poll write")?;
    let slot_keypair = community
        .slot_keypair
        .clone()
        .ok_or("slot keypair missing for poll write")?
        .parse::<veilid_core::KeyPair>()
        .map_err(|e| format!("invalid slot keypair: {e}"))?;
    Ok(PollWriteContext {
        channel_key: community
            .channel_log_keys
            .get(channel_id)
            .cloned()
            .ok_or("channel record key missing for poll write")?,
        slot_index: community
            .my_subkey_index
            .ok_or("subkey index missing for poll write")?,
        slot_keypair,
    })
}

fn validate_poll_create(question: &str, answers: &[String]) -> Result<(), String> {
    if question.trim().is_empty() {
        return Err("poll question cannot be empty".to_string());
    }
    if answers.len() < 2 || answers.len() > 10 {
        return Err("polls must have between 2 and 10 answers".to_string());
    }
    if answers.iter().any(|answer| answer.trim().is_empty()) {
        return Err("poll answers cannot be empty".to_string());
    }
    Ok(())
}

fn random_poll_id() -> [u8; 16] {
    use rand::RngCore;

    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

fn parse_poll_id(poll_id_hex: &str) -> Result<[u8; 16], String> {
    hex::decode(poll_id_hex)
        .map_err(|e| format!("invalid poll id hex: {e}"))?
        .try_into()
        .map_err(|_| "poll id must be 16 bytes".to_string())
}

fn dedupe_selected_answers(selected_answers: Vec<u8>) -> Vec<u8> {
    let mut selected_answers = selected_answers;
    selected_answers.sort_unstable();
    selected_answers.dedup();
    selected_answers
}

async fn write_poll_create_once(
    state: &SharedState,
    context: &PollWriteContext,
    entry: &ChannelPollCreate,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    write_member_poll_create(
        &mgr,
        &context.channel_key,
        context.slot_index,
        context.slot_keypair.clone(),
        entry,
    )
    .await
    .map_err(|e| format!("SMPL poll create write failed: {e}"))
}

async fn write_poll_vote_once(
    state: &SharedState,
    context: &PollWriteContext,
    entry: &ChannelPollVote,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    write_member_poll_vote(
        &mgr,
        &context.channel_key,
        context.slot_index,
        context.slot_keypair.clone(),
        entry,
    )
    .await
    .map_err(|e| format!("SMPL poll vote write failed: {e}"))
}

async fn write_poll_close_once(
    state: &SharedState,
    context: &PollWriteContext,
    entry: &ChannelPollClose,
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let mgr = rekindle_protocol::dht::DHTManager::new(rc);
    write_member_poll_close(
        &mgr,
        &context.channel_key,
        context.slot_index,
        context.slot_keypair.clone(),
        entry,
    )
    .await
    .map_err(|e| format!("SMPL poll close write failed: {e}"))
}

async fn ensure_poll_author(
    state: &SharedState,
    context: &PollWriteContext,
    poll_id: [u8; 16],
) -> Result<(), String> {
    let rc = state_helpers::safe_routing_context(state).ok_or("not attached")?;
    let entries = read_all_channel_entries(&rc, &context.channel_key, 255)
        .await
        .map_err(|e| format!("read channel poll history: {e}"))?;
    let mut author_subkey = None;
    let mut best_order = None;
    for item in entries {
        let ChannelRecordEntry::PollCreate(create) = item.entry else {
            continue;
        };
        if create.poll_id != poll_id {
            continue;
        }
        let order = (create.lamport, item.subkey_index);
        if best_order.is_none_or(|best| order >= best) {
            best_order = Some(order);
            author_subkey = Some(item.subkey_index);
        }
    }
    match author_subkey {
        Some(subkey) if subkey == context.slot_index => Ok(()),
        Some(_) => Err("only the poll author or a moderator can close this poll".to_string()),
        None => Err("poll not found".to_string()),
    }
}
