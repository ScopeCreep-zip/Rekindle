//! Channel message state — per-channel messages, typing, unread, threads, pins.
//!
//! Messages are stored in a `TrackedBuffer` — a VecDeque with automatic
//! generation tracking. Reactions and pins are stored separately and increment
//! `extra_generation`. The combined `generation()` is the sum of both counters,
//! monotonic under single-threaded access (guaranteed by the machine.rs select loop).

use std::collections::HashMap;
use std::time::Instant;

use rekindle_types::display::{DecryptedMessageDisplay, DeliveryStatus};

use super::tracked_buffer::TrackedBuffer;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ChannelKey {
    pub community: String,
    pub channel: String,
}

#[derive(Clone, Debug)]
pub struct PinDisplay {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    /// Milliseconds since Unix epoch.
    pub pinned_at: u64,
    pub body_preview: String,
}

/// Per-channel view state. Capped at 5000 messages with FIFO eviction.
///
/// All cache-invalidating data lives behind either `TrackedBuffer` (messages)
/// or `extra_generation` (reactions, pins). The combined `generation()` is
/// the staleness signal for the render cache.
#[derive(Debug)]
pub struct ChannelViewState {
    messages: TrackedBuffer<DecryptedMessageDisplay>,
    /// Reactions and pins increment this. Combined with messages.generation()
    /// in the `generation()` accessor.
    extra_generation: u64,
    /// UUID resolved from community info.
    pub channel_id: Option<String>,
    /// Pseudonym → timestamp. Expired at 5s by deadline check.
    pub typing_indicators: HashMap<String, Instant>,
    pub unread_count: u32,
    pub loaded: bool,
    pub loading: bool,
    pub loaded_at: Option<Instant>,
    /// Keyed by thread_id. Plain Vec — no generation tracking until a render
    /// cache is added for the thread panel. When that happens, migrate to
    /// TrackedBuffer<DecryptedMessageDisplay>.
    pub thread_messages: HashMap<String, Vec<DecryptedMessageDisplay>>,
    /// Peer key of the open split-pane DM (right side of channel watch).
    pub active_split_dm: Option<String>,
    pins: Option<Vec<PinDisplay>>,
    /// Keyed by message_id. Each entry is (emoji, count).
    reactions: HashMap<String, Vec<(String, u32)>>,
    pub channel_tree_selected: Option<usize>,
    pub peer_list_selected: Option<usize>,
}

impl ChannelViewState {
    pub const MAX_MESSAGES: usize = 5000;

    pub fn new() -> Self {
        Self {
            messages: TrackedBuffer::with_capacity(Self::MAX_MESSAGES),
            extra_generation: 0,
            channel_id: None,
            typing_indicators: HashMap::new(),
            unread_count: 0,
            loaded: false,
            loading: false,
            loaded_at: None,
            thread_messages: HashMap::new(),
            active_split_dm: None,
            pins: None,
            reactions: HashMap::new(),
            channel_tree_selected: None,
            peer_list_selected: None,
        }
    }

    // ── Read accessors ──────────────────────────────────────────

    /// Read-only access to messages as a VecDeque reference.
    pub fn messages(&self) -> &std::collections::VecDeque<DecryptedMessageDisplay> {
        self.messages.as_deque()
    }

    /// Combined generation: messages + reactions + pins.
    /// Single-threaded invariant: only one counter increments per mutation,
    /// so the sum is monotonically increasing.
    pub fn generation(&self) -> u64 {
        self.messages.generation() + self.extra_generation
    }

    pub fn reactions(&self) -> &HashMap<String, Vec<(String, u32)>> {
        &self.reactions
    }

    pub fn pins(&self) -> Option<&[PinDisplay]> {
        self.pins.as_deref()
    }

    pub fn has_pins(&self) -> bool {
        self.pins.as_ref().map_or(false, |p| !p.is_empty())
    }

    // ── Message mutation ────────────────────────────────────────

    /// Append a message, evicting the oldest if at capacity.
    /// If eviction occurs, also removes orphaned reactions for the evicted message.
    /// Returns `true` if eviction occurred.
    #[must_use]
    pub fn push_message(&mut self, msg: DecryptedMessageDisplay) -> bool {
        if let Some(evicted) = self.messages.push_capped(msg, Self::MAX_MESSAGES) {
            self.reactions.remove(&evicted.message_id);
            true
        } else {
            false
        }
    }

    /// Replace messages with historical data from daemon, preserving any
    /// optimistic sends or real-time messages that arrived after the history
    /// request was sent.
    pub fn set_messages(&mut self, msgs: std::collections::VecDeque<DecryptedMessageDisplay>) {
        let newest_ts = msgs.back().map(|m| m.timestamp).unwrap_or(0);
        self.messages.replace_preserving(msgs, |m| {
            m.delivery_status == DeliveryStatus::Sending
                || m.timestamp > newest_ts
        });
    }

    pub fn retain_messages<F: FnMut(&DecryptedMessageDisplay) -> bool>(&mut self, f: F) {
        self.messages.retain(f);
    }

    pub fn edit_message(&mut self, message_id: &str, new_body: String) {
        self.messages.find_mut_rev(
            |m| m.message_id == message_id,
            |m| {
                m.body = new_body;
                m.is_encrypted = false;
                m.needs_mek = None;
            },
        );
    }

    pub fn remove_message(&mut self, message_id: &str) {
        self.messages.retain(|m| m.message_id != message_id);
        self.reactions.remove(message_id);
    }

    /// Confirm the most recent Sending message (own messages have empty author_pseudonym).
    #[must_use]
    pub fn confirm_last_sending(&mut self, message_id: &str) -> bool {
        self.messages.find_mut_rev(
            |m| m.delivery_status == DeliveryStatus::Sending && m.author_pseudonym.is_empty(),
            |m| {
                m.delivery_status = DeliveryStatus::Confirmed;
                m.message_id = message_id.to_string();
            },
        )
    }

    /// Fail the most recent Sending message.
    pub fn fail_last_sending(&mut self) {
        self.messages.find_mut_rev(
            |m| m.delivery_status == DeliveryStatus::Sending,
            |m| m.delivery_status = DeliveryStatus::Failed,
        );
    }

    /// Fail the Sending message matching body text.
    pub fn fail_by_body(&mut self, body: &str, reply_to: Option<u64>) {
        self.messages.find_mut_rev(
            |m| m.delivery_status == DeliveryStatus::Sending
                && m.author_pseudonym.is_empty()
                && m.body == body,
            |m| {
                m.delivery_status = DeliveryStatus::Failed;
                if m.reply_to_sequence.is_none() {
                    m.reply_to_sequence = reply_to;
                }
            },
        );
    }

    /// Try to enrich a "(decrypting...)" placeholder.
    #[must_use]
    pub fn try_enrich_placeholder(
        &mut self,
        author: &str,
        timestamp: u64,
        message_id: &str,
        body: &str,
        sequence: u64,
        reply_to: Option<u64>,
    ) -> bool {
        self.messages.find_mut_rev(
            |m| m.author_pseudonym == author
                && m.body == "(decrypting...)"
                && m.timestamp.abs_diff(timestamp) < 5000,
            |m| {
                m.body = body.to_string();
                m.message_id = message_id.to_string();
                m.sequence = sequence;
                m.reply_to_sequence = reply_to;
                m.is_encrypted = false;
                m.needs_mek = None;
            },
        )
    }

    // ── Reaction mutation ───────────────────────────────────────

    pub fn add_reaction(&mut self, message_id: &str, emoji: &str) {
        let entry = self.reactions.entry(message_id.to_string()).or_default();
        if let Some(existing) = entry.iter_mut().find(|(e, _)| e == emoji) {
            existing.1 += 1;
        } else {
            entry.push((emoji.to_string(), 1));
        }
        self.extra_generation += 1;
    }

    pub fn remove_reaction(&mut self, message_id: &str, emoji: &str) {
        if let Some(entry) = self.reactions.get_mut(message_id) {
            if let Some(existing) = entry.iter_mut().find(|(e, _)| e == emoji) {
                existing.1 = existing.1.saturating_sub(1);
                if existing.1 == 0 {
                    entry.retain(|(e, _)| e != emoji);
                }
            }
            if entry.is_empty() {
                self.reactions.remove(message_id);
            }
        }
        self.extra_generation += 1;
    }

    // ── Pin mutation ────────────────────────────────────────────

    pub fn set_pins(&mut self, pins: Vec<PinDisplay>) {
        self.pins = Some(pins);
        self.extra_generation += 1;
    }

    pub fn clear_pins(&mut self) {
        self.pins = None;
        self.extra_generation += 1;
    }

    // ── Generation touch ────────────────────────────────────────

    /// Increment generation without modifying data.
    pub fn touch_generation(&mut self) {
        self.extra_generation += 1;
    }
}

impl Default for ChannelViewState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct ChannelDataState {
    pub channels: HashMap<ChannelKey, ChannelViewState>,
}

impl ChannelDataState {
    pub fn new() -> Self {
        Self { channels: HashMap::new() }
    }
}
