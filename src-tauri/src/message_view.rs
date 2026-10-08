//! Phase 23.C — pure message-list projection helpers lifted from
//! `commands/community/legacy/messages.rs`. Same shape as
//! `audit_view.rs` and `channel_materialize.rs`: a sibling
//! src-tauri-root module of pure functions consumed by commands +
//! services. No AppState, no Veilid, no SQL.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::commands::chat::{Message, ReactionGroup};

/// Merge `fallback` into `existing` (deduplicated by message_identity),
/// merge per-field updates for collisions, sort ascending by
/// timestamp, and truncate to the most-recent `limit` rows.
pub fn merge_message_lists(existing: &mut Vec<Message>, fallback: Vec<Message>, limit: u32) {
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
