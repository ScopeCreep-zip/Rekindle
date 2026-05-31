//! Phase 23.D.5 — channel polls orchestration ported from
//! `src-tauri/services/community/channel_polls.rs`. All protocol logic
//! (validate, dedup, snapshot aggregation, DHT write) lives here
//! parameterised over `ChannelMessagingDeps`; src-tauri retains a
//! thin facade.

use std::collections::HashMap;

use rand::RngCore;
use rekindle_protocol::dht::community::channel_record::{
    ChannelPollClose, ChannelPollCreate, ChannelPollVote, ChannelRecordEntry,
};

use crate::deps::ChannelMessagingDeps;
use crate::error::ChannelError;

/// Aggregated snapshot of a poll's state computed by walking the channel
/// record entries. The author_subkey is used to gate close authority;
/// the latest_votes map is keyed by voter subkey index so each member
/// counts once regardless of vote-replay attempts.
pub struct PollSnapshot {
    pub author_subkey: u32,
    pub answer_count: usize,
    pub multi_select: bool,
    pub expired: bool,
    pub closed: bool,
    pub latest_votes: HashMap<u32, Vec<u8>>,
}

/// Validate poll-create inputs. Pure — no deps.
pub fn validate_poll_create(question: &str, answers: &[String]) -> Result<(), ChannelError> {
    if question.trim().is_empty() {
        return Err(ChannelError::Adapter(
            "poll question cannot be empty".to_string(),
        ));
    }
    if answers.len() < 2 || answers.len() > 10 {
        return Err(ChannelError::Adapter(
            "polls must have between 2 and 10 answers".to_string(),
        ));
    }
    if answers.iter().any(|answer| answer.trim().is_empty()) {
        return Err(ChannelError::Adapter(
            "poll answers cannot be empty".to_string(),
        ));
    }
    Ok(())
}

#[must_use]
pub fn random_poll_id() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

pub fn parse_poll_id(poll_id_hex: &str) -> Result<[u8; 16], ChannelError> {
    hex::decode(poll_id_hex)
        .map_err(|e| ChannelError::Adapter(format!("invalid poll id hex: {e}")))?
        .try_into()
        .map_err(|_| ChannelError::Adapter("poll id must be 16 bytes".to_string()))
}

#[must_use]
pub fn dedupe_selected_answers(selected_answers: Vec<u8>) -> Vec<u8> {
    let mut selected_answers = selected_answers;
    selected_answers.sort_unstable();
    selected_answers.dedup();
    selected_answers
}

pub async fn persist_poll_create<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    message_id: &str,
    question: &str,
    answers: Vec<String>,
    multi_select: bool,
    duration_seconds: Option<u64>,
) -> Result<String, ChannelError> {
    validate_poll_create(question, &answers)?;
    let context = deps.channel_write_context(community_id, channel_id)?;
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
        expires_at: duration_seconds
            .map(|seconds| rekindle_utils::timestamp_secs().saturating_add(seconds)),
        lamport: deps.increment_lamport(community_id),
    };
    deps.write_channel_poll_create_smpl(&context, &entry).await?;
    Ok(hex::encode(poll_id))
}

pub async fn persist_poll_vote<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    selected_answers: Vec<u8>,
) -> Result<(), ChannelError> {
    if selected_answers.is_empty() {
        return Err(ChannelError::Adapter(
            "at least one answer must be selected".to_string(),
        ));
    }
    let context = deps.channel_write_context(community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    validate_poll_vote(deps, &context.channel_key, poll_id, &selected_answers).await?;
    let entry = ChannelPollVote {
        poll_id,
        selected_answers: dedupe_selected_answers(selected_answers),
        lamport: deps.increment_lamport(community_id),
    };
    deps.write_channel_poll_vote_smpl(&context, &entry).await
}

pub async fn persist_poll_close<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
    allow_moderator_override: bool,
) -> Result<(), ChannelError> {
    let context = deps.channel_write_context(community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    if !allow_moderator_override {
        ensure_poll_author(deps, &context.channel_key, context.slot_index, poll_id).await?;
    }
    let entry = ChannelPollClose {
        poll_id,
        lamport: deps.increment_lamport(community_id),
    };
    deps.write_channel_poll_close_smpl(&context, &entry).await
}

pub async fn get_poll_results<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    poll_id_hex: &str,
) -> Result<Vec<u32>, ChannelError> {
    let context = deps.channel_write_context(community_id, channel_id)?;
    let poll_id = parse_poll_id(poll_id_hex)?;
    let snapshot = load_poll_snapshot(deps, &context.channel_key, poll_id).await?;
    let mut counts = vec![0_u32; snapshot.answer_count];
    for selected in snapshot.latest_votes.into_values() {
        for index in selected {
            if let Some(count) = counts.get_mut(usize::from(index)) {
                *count = count.saturating_add(1);
            }
        }
    }
    Ok(counts)
}

async fn ensure_poll_author<D: ChannelMessagingDeps>(
    deps: &D,
    channel_key: &str,
    slot_index: u32,
    poll_id: [u8; 16],
) -> Result<(), ChannelError> {
    let snapshot = load_poll_snapshot(deps, channel_key, poll_id).await?;
    if snapshot.author_subkey == slot_index {
        Ok(())
    } else {
        Err(ChannelError::Adapter(
            "only the poll author or a moderator can close this poll".to_string(),
        ))
    }
}

async fn validate_poll_vote<D: ChannelMessagingDeps>(
    deps: &D,
    channel_key: &str,
    poll_id: [u8; 16],
    selected_answers: &[u8],
) -> Result<(), ChannelError> {
    let snapshot = load_poll_snapshot(deps, channel_key, poll_id).await?;
    if snapshot.closed {
        return Err(ChannelError::Adapter("poll is closed".to_string()));
    }
    if snapshot.expired {
        return Err(ChannelError::Adapter("poll has expired".to_string()));
    }
    if !snapshot.multi_select && selected_answers.len() > 1 {
        return Err(ChannelError::Adapter(
            "poll allows only one answer".to_string(),
        ));
    }
    if selected_answers
        .iter()
        .any(|index| usize::from(*index) >= snapshot.answer_count)
    {
        return Err(ChannelError::Adapter(
            "selected answer is out of range".to_string(),
        ));
    }
    Ok(())
}

async fn load_poll_snapshot<D: ChannelMessagingDeps>(
    deps: &D,
    channel_key: &str,
    poll_id: [u8; 16],
) -> Result<PollSnapshot, ChannelError> {
    let entries = deps.read_all_channel_entries(channel_key, 255).await?;

    let mut author_subkey = None;
    let mut best_create_order = None;
    let mut answer_count = 0_usize;
    let mut multi_select = false;
    let mut expires_at = None;
    let mut closed_lamport = None;
    let mut latest_votes: HashMap<u32, Vec<u8>> = HashMap::new();

    for item in &entries {
        let ChannelRecordEntry::PollCreate(create) = &item.entry else {
            continue;
        };
        if create.poll_id != poll_id {
            continue;
        }
        let order = (create.lamport, item.subkey_index);
        let should_replace = match (best_create_order, author_subkey) {
            (None, _) => true,
            (Some(best_order), Some(existing_author_subkey)) => {
                existing_author_subkey == item.subkey_index && order >= best_order
            }
            _ => false,
        };
        if should_replace {
            best_create_order = Some(order);
            author_subkey = Some(item.subkey_index);
            answer_count = create.answers.len();
            multi_select = create.multi_select;
            expires_at = create.expires_at;
        }
    }

    let Some(author_subkey) = author_subkey else {
        return Err(ChannelError::Adapter("poll not found".to_string()));
    };

    for item in &entries {
        match &item.entry {
            ChannelRecordEntry::PollClose(close)
                if close.poll_id == poll_id && item.subkey_index == author_subkey =>
            {
                if closed_lamport.is_none_or(|lamport| close.lamport >= lamport) {
                    closed_lamport = Some(close.lamport);
                }
            }
            ChannelRecordEntry::PollVote(vote) if vote.poll_id == poll_id => {
                if closed_lamport.is_some_and(|lamport| vote.lamport > lamport) {
                    continue;
                }
                latest_votes.insert(
                    item.subkey_index,
                    dedupe_selected_answers(vote.selected_answers.clone()),
                );
            }
            _ => {}
        }
    }

    Ok(PollSnapshot {
        author_subkey,
        answer_count,
        multi_select,
        expired: expires_at.is_some_and(|ts| ts <= rekindle_utils::timestamp_secs()),
        closed: closed_lamport.is_some(),
        latest_votes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_poll_create_rejects_empty_question() {
        let err = validate_poll_create("   ", &["a".into(), "b".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
    }

    #[test]
    fn validate_poll_create_rejects_too_few_answers() {
        let err = validate_poll_create("q", &["only".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
    }

    #[test]
    fn validate_poll_create_rejects_too_many_answers() {
        let answers: Vec<String> = (0..11).map(|i| format!("a{i}")).collect();
        let err = validate_poll_create("q", &answers).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("2 and 10")));
    }

    #[test]
    fn validate_poll_create_rejects_empty_answer() {
        let err = validate_poll_create("q", &["a".into(), "  ".into()]).unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(msg) if msg.contains("empty")));
    }

    #[test]
    fn validate_poll_create_accepts_valid_input() {
        let result = validate_poll_create("Best color?", &["red".into(), "blue".into()]);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_poll_id_rejects_short_hex() {
        let err = parse_poll_id("abcd").unwrap_err();
        assert!(matches!(err, ChannelError::Adapter(_)));
    }

    #[test]
    fn parse_poll_id_accepts_32_hex_chars() {
        let id = parse_poll_id("00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(
            id,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
    }

    #[test]
    fn dedupe_selected_answers_sorts_and_dedups() {
        assert_eq!(dedupe_selected_answers(vec![2, 0, 1, 0, 2]), vec![0, 1, 2]);
    }

    #[test]
    fn random_poll_id_returns_16_bytes() {
        let id = random_poll_id();
        assert_eq!(id.len(), 16);
    }
}
