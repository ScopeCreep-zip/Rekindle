//! The D26 ingest gate (plan E3.1 F3): every received channel post passes
//! [`gate`] before it is stored, displayed or notified, on both hosts. It
//! checks the block list, bans, timeouts, the channel's slowmode and the
//! community's automod rules, in that order, and answers [`Verdict::Admit`],
//! [`Verdict::Report`] (admitted, and reported to the moderators),
//! [`Verdict::Hide`] (stored blurred) or [`Verdict::Drop`] (never stored).
//! An automod rule that cannot be evaluated drops the post: a broken rule
//! never admits what it was written to stop.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_codec::community::channel_record::ChannelMessage;
use rekindle_governance::permissions::{compute_permissions, has_capability};
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::permissions::BYPASS_SLOWMODE;

use crate::automod::{compile_rules, rule_fingerprint, rule_matches};
use crate::deps::ChannelMessagingDeps;
use crate::send::BodyPosition;

/// One received post, as the gate judges it.
#[derive(Debug, Clone, Copy)]
pub struct GateItem<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    /// The author's community pseudonym (hex).
    pub sender_pseudonym: &'a str,
    /// The opened body.
    pub body: &'a str,
    /// The post's own timestamp, in seconds: slowmode compares a sender's
    /// posts by it, so every peer reaches the same verdict.
    pub sent_at_secs: u64,
    /// The receiver's clock, in seconds: a timeout ends by it.
    pub now_secs: u64,
}

/// Why the gate hid or dropped a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateReason {
    /// We blocked the author.
    Blocked,
    /// The author is banned from the community.
    Banned,
    /// The author is timed out until the given second.
    TimedOut { until_secs: u64 },
    /// The author posted again within the channel's slowmode.
    Slowmode { seconds: u32 },
    /// An automod rule matched.
    AutoMod { rule: String },
    /// The automod rules could not be evaluated.
    AutoModError(String),
}

/// The gate's answer for one post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Stored and shown.
    Admit,
    /// Stored and shown, and reported to the moderators.
    Report(GateReason),
    /// Stored, shown blurred.
    Hide(GateReason),
    /// Never stored.
    Drop(GateReason),
}

/// The keys we blocked (`rekindle_db::repo::blocked`), lowercased.
#[derive(Debug, Clone, Default)]
pub struct BlockList(BTreeSet<String>);

impl BlockList {
    /// The block list of `keys`.
    pub fn new(keys: impl IntoIterator<Item = String>) -> Self {
        Self(keys.into_iter().map(|k| k.to_ascii_lowercase()).collect())
    }

    /// Whether `key` is blocked.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains(&key.to_ascii_lowercase())
    }
}

/// The last admitted post's time per community, channel and author, for
/// slowmode.
#[derive(Debug, Default)]
pub struct RateLimits {
    last: BTreeMap<(String, String, String), u64>,
}

/// Judge `item` against the community's merged state `gov`, the block list
/// and the slowmode windows; an admitted or hidden post starts its author's
/// next window.
pub fn gate(
    item: &GateItem<'_>,
    gov: &GovernanceState,
    blocked: &BlockList,
    limits: &mut RateLimits,
) -> Verdict {
    if blocked.contains(item.sender_pseudonym) {
        return Verdict::Drop(GateReason::Blocked);
    }
    let sender = PseudonymKey::from_hex_lossy(item.sender_pseudonym);
    if gov.bans.contains(&sender) {
        return Verdict::Drop(GateReason::Banned);
    }
    if let Some(timeout) = gov.timeouts.get(&sender) {
        let until_secs = timeout.started_at.saturating_add(timeout.duration_seconds);
        if item.now_secs < until_secs {
            return Verdict::Drop(GateReason::TimedOut { until_secs });
        }
    }
    let channel = ChannelId::from_hex(item.channel_id);
    if let Some(seconds) = slowmode_seconds(gov, channel.as_ref()) {
        let perms = compute_permissions(&sender, channel.as_ref(), gov, item.now_secs);
        if !has_capability(perms, BYPASS_SLOWMODE) && within_window(item, limits, seconds) {
            return Verdict::Drop(GateReason::Slowmode { seconds });
        }
    }
    let verdict = automod(gov, item.body);
    if !matches!(verdict, Verdict::Drop(_)) {
        limits.last.insert(window_key(item), item.sent_at_secs);
    }
    verdict
}

fn slowmode_seconds(gov: &GovernanceState, channel: Option<&ChannelId>) -> Option<u32> {
    channel
        .and_then(|id| gov.channels.get(id))
        .and_then(|ch| ch.slowmode_seconds)
        .filter(|s| *s > 0)
}

fn window_key(item: &GateItem<'_>) -> (String, String, String) {
    (
        item.community_id.to_owned(),
        item.channel_id.to_owned(),
        item.sender_pseudonym.to_ascii_lowercase(),
    )
}

fn within_window(item: &GateItem<'_>, limits: &RateLimits, seconds: u32) -> bool {
    limits
        .last
        .get(&window_key(item))
        .is_some_and(|last| item.sent_at_secs.saturating_sub(*last) < u64::from(seconds))
}

/// The automod verdict: a matching `block_locally` rule drops, then
/// `blur_content` hides, then `alert_moderators` admits with a report. Rules are taken in
/// rule-id order, so the rule named is the same on every peer.
fn automod(gov: &GovernanceState, body: &str) -> Verdict {
    let compiled = match compile_rules(gov, &rule_fingerprint(gov)) {
        Ok(compiled) => compiled,
        Err(e) => return Verdict::Drop(GateReason::AutoModError(e.to_string())),
    };
    let mut rules = compiled.rules;
    rules.sort_by_key(|rule| rule.rule_id);
    let mut verdict = Verdict::Admit;
    for rule in rules.iter().filter(|rule| rule_matches(rule, body)) {
        let reason = GateReason::AutoMod {
            rule: rule.name.clone(),
        };
        match rule.action.as_str() {
            "block_locally" => return Verdict::Drop(reason),
            "blur_content" if !matches!(verdict, Verdict::Hide(_)) => {
                verdict = Verdict::Hide(reason);
            }
            "alert_moderators" if verdict == Verdict::Admit => verdict = Verdict::Report(reason),
            "blur_content" | "alert_moderators" => {}
            other => {
                return Verdict::Drop(GateReason::AutoModError(format!(
                    "rule {:?} has the unknown action {other:?}",
                    rule.name
                )))
            }
        }
    }
    verdict
}

/// Open `message`'s body, written at `subkey_index` of `record_key`, under
/// the generation it names; `None` when we hold no key that opens it.
pub fn open_body<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    record_key: &str,
    subkey_index: u32,
    message: &ChannelMessage,
) -> Option<String> {
    crate::text_keys::open_text(
        deps,
        community_id,
        channel_id,
        message.mek_generation,
        BodyPosition {
            channel_record_key: record_key,
            subkey_index,
            lamport_ts: message.lamport_ts,
        },
        &message.ciphertext,
    )
    .map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
#[path = "ingest_tests.rs"]
mod tests;
