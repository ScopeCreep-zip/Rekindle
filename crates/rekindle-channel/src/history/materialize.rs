//! A channel's reactions as messages carry them (plan E3.3, rules R1–R4),
//! aggregated from the reactors' outbox entries. Pure.

use std::collections::{BTreeMap, HashMap};

use rekindle_codec::community::channel_record::ChannelRecordEntry;
use rekindle_types::dto::channel_message::ReactionGroup;

use crate::deps::OutboxItem;

/// The most distinct reactions one message carries (rule R2; Discord's
/// limit of 20 unique reactions per message): further emojis are not
/// counted.
pub const MAX_REACTIONS_PER_MESSAGE: usize = 20;

/// The longest reaction key, bytes (rule R2): a `custom:` expression id
/// is 39, the longest emoji ZWJ sequences stay under it.
pub const MAX_REACTION_KEY_LEN: usize = 64;

/// The reactions of a channel's outbox entries by target message id (rules
/// R1–R3): per (reactor, target, emoji) the reactor's latest op by its
/// stream seq wins and a repeat changes nothing (R1); a key over
/// [`MAX_REACTION_KEY_LEN`] is dropped and a message counts its first
/// [`MAX_REACTIONS_PER_MESSAGE`] emojis by first add (R2); the aggregate is
/// the reader's own, from the reactors' outboxes (R3), and each group names
/// its reactors (R4).
#[must_use]
pub fn build_reaction_groups(entries: &[OutboxItem]) -> HashMap<String, Vec<ReactionGroup>> {
    // (target, emoji, reactor) -> (stream seq, added, committed at)
    let mut latest: HashMap<(String, String, String), (u64, bool, u64)> = HashMap::new();
    for item in entries {
        let ChannelRecordEntry::Reaction(reaction) = &item.entry else {
            continue;
        };
        if reaction.expression.is_empty() || reaction.expression.len() > MAX_REACTION_KEY_LEN {
            continue;
        }
        let key = (
            reaction.message_id.clone(),
            reaction.expression.clone(),
            hex::encode(item.author.0),
        );
        let replace = (latest.get(&key)).is_none_or(|(seq, _, _)| item.stream_seq > *seq);
        if replace {
            latest.insert(key, (item.stream_seq, reaction.added, item.committed_at));
        }
    }
    // (target, emoji) -> (first add, reactors)
    let mut grouped: BTreeMap<(String, String), ((u64, String), Vec<String>)> = BTreeMap::new();
    for ((target, emoji, reactor), (_, added, at)) in latest {
        if !added {
            continue;
        }
        let group =
            (grouped.entry((target, emoji))).or_insert_with(|| ((at, reactor.clone()), Vec::new()));
        group.0 = group.0.clone().min((at, reactor.clone()));
        group.1.push(reactor);
    }
    let mut by_message: HashMap<String, Vec<((u64, String), ReactionGroup)>> = HashMap::new();
    for ((target, emoji), (first, mut reactors)) in grouped {
        reactors.sort();
        let group = ReactionGroup {
            emoji,
            count: u32::try_from(reactors.len()).unwrap_or(u32::MAX),
            reactors,
        };
        by_message.entry(target).or_default().push((first, group));
    }
    by_message
        .into_iter()
        .map(|(target, mut groups)| {
            groups.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.emoji.cmp(&b.1.emoji)));
            groups.truncate(MAX_REACTIONS_PER_MESSAGE);
            (target, groups.into_iter().map(|(_, group)| group).collect())
        })
        .collect()
}

#[cfg(test)]
#[path = "materialize_reaction_tests.rs"]
mod reaction_tests;
