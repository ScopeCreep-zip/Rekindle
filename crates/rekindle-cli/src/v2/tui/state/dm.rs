//! DM conversation state — all thread data, keyed by peer.
//!
//! Messages are stored in a `TrackedBuffer` — a VecDeque with automatic
//! generation tracking. Every mutation auto-increments generation. Render caches
//! compare generation to determine staleness.

use std::collections::VecDeque;
use std::time::Instant;

use indexmap::IndexMap;
use rekindle_types::display::{DeliveryStatus, DmMessageDisplay};

use super::tracked_buffer::TrackedBuffer;

/// DM message with TUI-local delivery status for optimistic send tracking.
#[derive(Clone, Debug)]
pub struct DmMessage {
    pub display: DmMessageDisplay,
    pub delivery_status: DeliveryStatus,
}

/// Per-peer DM thread. Single source of truth for both inbox and thread views.
#[derive(Debug)]
pub struct DmThreadState {
    pub peer_key: String,
    pub peer_name: String,
    messages: TrackedBuffer<DmMessage>,
    pub unread_count: u32,
    /// Milliseconds since Unix epoch.
    pub last_message_at: Option<u64>,
    pub is_group: bool,
    pub loaded: bool,
    pub loading: bool,
    pub is_typing: bool,
    /// For local 5s expiry.
    pub typing_since: Option<Instant>,
}

impl DmThreadState {
    /// Maximum messages kept per DM thread. FIFO eviction at capacity.
    pub const MAX_MESSAGES: usize = 5000;

    pub fn new(peer_key: String, peer_name: String) -> Self {
        Self {
            peer_key,
            peer_name,
            messages: TrackedBuffer::with_capacity(Self::MAX_MESSAGES),
            unread_count: 0,
            last_message_at: None,
            is_group: false,
            loaded: false,
            loading: false,
            is_typing: false,
            typing_since: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.messages.generation()
    }

    /// Read-only access to the message buffer as a VecDeque reference.
    pub fn messages(&self) -> &VecDeque<DmMessage> {
        self.messages.as_deque()
    }

    /// Append a message, evicting the oldest if at capacity.
    pub fn push_message(&mut self, msg: DmMessage) {
        let _ = self.messages.push_capped(msg, Self::MAX_MESSAGES);
    }

    /// Replace messages with historical data from daemon, preserving any
    /// optimistic sends or real-time messages that arrived after the history
    /// request was sent.
    pub fn set_messages(&mut self, msgs: Vec<DmMessage>) {
        let newest_ts = msgs.last().map(|m| m.display.timestamp).unwrap_or(0);
        self.messages.replace_preserving(msgs, |m| {
            m.delivery_status == DeliveryStatus::Sending
                || m.display.timestamp > newest_ts
        });
    }

    /// Retain only messages matching the predicate.
    pub fn retain_messages<F: FnMut(&DmMessage) -> bool>(&mut self, f: F) {
        self.messages.retain(f);
    }

    /// Confirm the most recent Sending message matching body. Returns true if found.
    #[must_use]
    pub fn confirm_by_body(&mut self, body: &str) -> bool {
        self.messages.find_mut_rev(
            |m| m.display.is_self
                && m.delivery_status == DeliveryStatus::Sending
                && m.display.body == body,
            |m| m.delivery_status = DeliveryStatus::Confirmed,
        )
    }

    /// Fail the most recent Sending message.
    pub fn fail_last_sending(&mut self) {
        self.messages.find_mut_rev(
            |m| m.display.is_self && m.delivery_status == DeliveryStatus::Sending,
            |m| m.delivery_status = DeliveryStatus::Failed,
        );
    }

    /// Try to enrich a "(decrypting...)" placeholder by timestamp proximity.
    #[must_use]
    pub fn try_enrich_placeholder(
        &mut self,
        timestamp: u64,
        body: &str,
        sender_name: Option<&str>,
    ) -> bool {
        self.messages.find_mut_rev(
            |m| m.display.body == "(decrypting...)"
                && m.display.timestamp.abs_diff(timestamp) < 5000,
            |m| {
                m.display.body = body.to_string();
                if let Some(name) = sender_name {
                    m.display.sender_name = name.to_string();
                }
            },
        )
    }
}

/// All DM state. Sorted by `last_message_at` descending. O(1) key lookup.
#[derive(Debug)]
pub struct DmState {
    pub threads: IndexMap<String, DmThreadState>,
    pub inbox_loaded: bool,
    pub inbox_loaded_at: Option<Instant>,
}

impl DmState {
    pub fn new() -> Self {
        Self {
            threads: IndexMap::new(),
            inbox_loaded: false,
            inbox_loaded_at: None,
        }
    }

    /// Re-sort threads by `last_message_at` descending (most recent first).
    pub fn sort_by_last_message(&mut self) {
        self.threads.sort_by(|_, a, _, b| {
            b.last_message_at.cmp(&a.last_message_at)
        });
    }
}

impl Default for DmState {
    fn default() -> Self {
        Self::new()
    }
}
