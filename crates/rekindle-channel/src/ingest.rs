//! The D26 ingest gate (plan E3.1 F3): every received channel post passes
//! [`gate`] before it is stored, displayed or notified, on both hosts. It
//! checks the block list, bans, timeouts, the channel's slowmode and the
//! community's automod rules, in that order, and answers [`Verdict::Admit`],
//! [`Verdict::Report`] (admitted, and reported to the moderators),
//! [`Verdict::Hide`] (stored blurred) or [`Verdict::Drop`] (never stored).
//! An automod rule that cannot be evaluated drops the post: a broken rule
//! never admits what it was written to stop.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::BuildHasher;

use rekindle_codec::community::channel_record::ChannelMessage;
use rekindle_governance::permissions::{compute_permissions, has_capability};
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};
use rekindle_types::permissions::BYPASS_SLOWMODE;

use rekindle_types::governance::AutoModAction;

use crate::automod::{rule_matches, AutoModCompiledCache};
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
    /// The author's nearest post in this channel among the posts we hold
    /// (stored, or admitted earlier in the same batch), at or before
    /// `sent_at_secs`, in seconds; the post itself is not one. Slowmode
    /// judges against it, never against the last post judged, so a
    /// backfilled older post gets the verdict the live path gave it in
    /// order (plan E3.1 FX2). A predecessor that arrives after its
    /// successor does not drop the successor retroactively.
    pub held_before_secs: Option<u64>,
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

/// The pseudonyms we block in the post's community
/// (`rekindle_db::repo::community_blocks`), lowercased.
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

/// The posts one batch admitted or hid before they are stored, per
/// author (lowercased), so a later post of the same batch is judged
/// against them as it would be against stored ones. It lives for one
/// batch and is discarded with it.
#[derive(Debug, Default)]
pub struct BatchWindows {
    held: BTreeMap<String, BTreeSet<u64>>,
}

impl BatchWindows {
    /// The author's nearest post at or before `at_secs`: the later of
    /// `stored` (the nearest stored one) and this batch's nearest.
    #[must_use]
    pub fn held_before(&self, author: &str, at_secs: u64, stored: Option<u64>) -> Option<u64> {
        let batch = (self.held.get(&author.to_ascii_lowercase()))
            .and_then(|times| times.range(..=at_secs).next_back().copied());
        stored.max(batch)
    }

    /// Record `author`'s post at `at_secs` of the batch, judged `verdict`:
    /// unless it was dropped it is held, so it starts a window of its
    /// author's.
    pub fn record(&mut self, author: &str, at_secs: u64, verdict: &Verdict) {
        if !matches!(verdict, Verdict::Drop(_)) {
            (self.held.entry(author.to_ascii_lowercase()))
                .or_default()
                .insert(at_secs);
        }
    }
}

/// When `member`'s timeout ends, while it lasts at `now_secs` (the
/// caller's clock port). The merge never reads a clock (thread validation
/// runs at time 0), so the Host refuses a timed-out member's thread at its
/// send path and holds one it receives ([`hold_timed_out_threads`]) by this.
#[must_use]
pub fn timed_out_until(gov: &GovernanceState, member: &PseudonymKey, now_secs: u64) -> Option<u64> {
    let timeout = gov.timeouts.get(member)?;
    let until_secs = timeout.started_at.saturating_add(timeout.duration_seconds);
    (now_secs < until_secs).then_some(until_secs)
}

/// Hold, at receipt, every thread of the merged `state` that is new to us
/// (in neither `previous` nor `held`) and whose creator is timed out at
/// `now_secs`; a held thread stays out of `state` for the session. A
/// `ThreadCreated` carries no time and the merge reads no clock, so the
/// moment we receive it is the only time we can judge. Replies are not
/// judged by their thread's creator: each post passes [`gate`] on its own
/// author. Returns the threads newly held, in id order, for the caller to
/// keep across a restart.
pub fn hold_timed_out_threads<S: BuildHasher>(
    previous: Option<&GovernanceState>,
    held: &mut HashSet<ThreadId, S>,
    state: &mut GovernanceState,
    now_secs: u64,
) -> Vec<ThreadId> {
    let fresh: Vec<ThreadId> = state
        .threads
        .iter()
        .filter(|(id, _)| !held.contains(*id))
        .filter(|(id, _)| !previous.is_some_and(|p| p.threads.contains_key(*id)))
        .filter(|(_, t)| timed_out_until(state, &t.creator, now_secs).is_some())
        .map(|(id, _)| *id)
        .collect();
    let mut fresh = fresh;
    fresh.sort_unstable_by_key(|id| id.0);
    held.extend(fresh.iter().copied());
    state.threads.retain(|id, _| !held.contains(id));
    fresh
}

/// Judge `item` against the community's merged state `gov`, its compiled
/// automod rules (`crate::automod::compiled_rules`), the block list and the
/// channel's slowmode against `item.held_before_secs`.
pub fn gate(
    item: &GateItem<'_>,
    gov: &GovernanceState,
    rules: &AutoModCompiledCache,
    blocked: &BlockList,
) -> Verdict {
    if blocked.contains(item.sender_pseudonym) {
        return Verdict::Drop(GateReason::Blocked);
    }
    let sender = PseudonymKey::from_hex_lossy(item.sender_pseudonym);
    if gov.bans.contains(&sender) {
        return Verdict::Drop(GateReason::Banned);
    }
    if let Some(until_secs) = timed_out_until(gov, &sender, item.now_secs) {
        return Verdict::Drop(GateReason::TimedOut { until_secs });
    }
    let channel = ChannelId::from_hex(item.channel_id);
    if let Some(seconds) = slowmode_seconds(gov, channel.as_ref()) {
        let perms = compute_permissions(&sender, channel.as_ref(), gov, item.now_secs);
        if !has_capability(perms, BYPASS_SLOWMODE) && within_window(item, seconds) {
            return Verdict::Drop(GateReason::Slowmode { seconds });
        }
    }
    automod(rules, item.body)
}

fn slowmode_seconds(gov: &GovernanceState, channel: Option<&ChannelId>) -> Option<u32> {
    channel
        .and_then(|id| gov.channels.get(id))
        .and_then(|ch| ch.slowmode_seconds)
        .filter(|s| *s > 0)
}

/// Whether the author's nearest held post at or before `item` is less than
/// `seconds` older than it.
fn within_window(item: &GateItem<'_>, seconds: u32) -> bool {
    (item
        .held_before_secs
        .filter(|earlier| *earlier <= item.sent_at_secs))
    .is_some_and(|earlier| item.sent_at_secs - earlier < u64::from(seconds))
}

/// The automod verdict: a matching `BlockLocally` rule drops, then
/// `BlurContent` hides, then `AlertModerators` admits with a report. The
/// compiled rules are in rule-id order, so the rule named is the same on
/// every peer.
fn automod(rules: &AutoModCompiledCache, body: &str) -> Verdict {
    let rules = match &rules.rules {
        Ok(rules) => rules,
        Err(e) => return Verdict::Drop(GateReason::AutoModError(e.clone())),
    };
    let mut verdict = Verdict::Admit;
    for rule in rules.iter().filter(|rule| rule_matches(rule, body)) {
        let reason = GateReason::AutoMod {
            rule: rule.name.clone(),
        };
        match rule.action {
            AutoModAction::BlockLocally => return Verdict::Drop(reason),
            AutoModAction::BlurContent if !matches!(verdict, Verdict::Hide(_)) => {
                verdict = Verdict::Hide(reason);
            }
            AutoModAction::AlertModerators if verdict == Verdict::Admit => {
                verdict = Verdict::Report(reason);
            }
            AutoModAction::BlurContent | AutoModAction::AlertModerators => {}
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
