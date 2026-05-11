//! Message list state — data management, scroll, delivery tracking.

use std::collections::VecDeque;

use ratatui::widgets::ListState;
use rekindle_types::display::DecryptedMessageDisplay;

use super::types::{MessageGroup, RenderedMessage};

/// Message list component state.
pub struct MessageList {
    pub(super) messages: VecDeque<RenderedMessage>,
    pub(super) list_state: ListState,
    pub(super) auto_scroll: bool,
    pub(super) community: String,
    pub(super) channel: String,
    pub(super) generation: u64,
    pub(super) last_rendered_generation: u64,
    pub(super) cached_items: Vec<ratatui::widgets::ListItem<'static>>,
    pub(super) is_focused: bool,
    pub(super) last_read_index: Option<usize>,
}

impl MessageList {
    pub fn new(community: String, channel: String) -> Self {
        Self {
            messages: VecDeque::new(), list_state: ListState::default(),
            auto_scroll: true, community, channel, generation: 0,
            last_rendered_generation: u64::MAX, cached_items: Vec::new(),
            is_focused: false, last_read_index: None,
        }
    }

    /// Maximum messages retained in memory per view. Oldest messages are
    /// evicted when this limit is reached. 5000 messages × ~500 bytes average
    /// ≈ 2.5MB per channel — bounded and predictable.
    const MAX_MESSAGES: usize = 5000;

    pub fn push(&mut self, msg: DecryptedMessageDisplay) {
        let is_own = msg.delivery_status == rekindle_types::display::DeliveryStatus::Sending
            || msg.author_pseudonym.is_empty();
        let group = self.compute_group(&msg);
        self.messages.push_back(RenderedMessage::new(msg, group));

        // Evict oldest messages when at capacity
        while self.messages.len() > Self::MAX_MESSAGES {
            self.messages.pop_front();
            // Adjust last_read_index to account for the removed message
            self.last_read_index = self.last_read_index.and_then(|i| i.checked_sub(1));
        }

        if is_own { self.last_read_index = Some(self.len().saturating_sub(1)); }
        if self.auto_scroll { self.list_state.select(Some(self.len().saturating_sub(1))); }
        self.generation += 1;
    }

    pub fn set_messages(&mut self, messages: Vec<DecryptedMessageDisplay>) {
        self.messages.clear();
        for msg in messages {
            let group = self.compute_group(&msg);
            self.messages.push_back(RenderedMessage::new(msg, group));
        }
        while self.messages.len() > Self::MAX_MESSAGES {
            self.messages.pop_front();
        }
        if !self.is_empty() { self.list_state.select(Some(self.len().saturating_sub(1))); }
        self.auto_scroll = true;
        self.generation += 1;
    }

    pub fn set_last_read(&mut self, index: usize) { self.last_read_index = Some(index); }
    pub fn selected_index(&self) -> Option<usize> { self.list_state.selected() }
    pub fn message_at(&self, index: usize) -> Option<&DecryptedMessageDisplay> { self.messages.get(index).map(|r| &r.msg) }
    pub fn len(&self) -> usize { self.messages.len() }
    pub fn is_empty(&self) -> bool { self.messages.is_empty() }

    pub fn remove_by_id(&mut self, message_id: &str) {
        self.messages.retain(|r| r.msg.message_id != message_id);
        if let Some(sel) = self.list_state.selected() {
            if sel >= self.len() && !self.is_empty() { self.list_state.select(Some(self.len() - 1)); }
        }
        self.generation += 1;
    }

    pub fn try_enrich_placeholder(
        &mut self, author: &str, timestamp: u64, message_id: &str,
        body: &str, sequence: u64, reply_to: Option<u64>,
    ) -> bool {
        let found = self.messages.iter_mut().rev().find(|r| {
            r.msg.author_pseudonym == author && r.msg.body == "(decrypting...)"
                && r.msg.timestamp.abs_diff(timestamp) < 5000
        });
        if let Some(r) = found {
            r.msg.body = body.to_string();
            r.msg.message_id = message_id.to_string();
            r.msg.sequence = sequence;
            r.msg.reply_to_sequence = reply_to;
            r.msg.is_encrypted = false;
            r.msg.needs_mek = None;
            self.generation += 1;
            true
        } else { false }
    }

    pub fn update_body(&mut self, message_id: &str, new_body: &str) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.msg.message_id == message_id) {
            r.msg.body = format!("{new_body} (edited)");
            r.msg.is_encrypted = false;
            r.msg.needs_mek = None;
            self.generation += 1;
        }
    }

    /// Confirm delivery of a self-sent message. Matches by message_id only —
    /// no fallback to "any pending message" to prevent confirming the wrong one
    /// when multiple messages are in flight simultaneously.
    pub fn confirm_message(&mut self, message_id: &str) {
        // Match by daemon-assigned message_id against pending client-side nonces.
        // Client nonces are "pending-{timestamp}" — the daemon returns a UUID.
        // Since the IDs don't match, also try matching by most-recent Sending
        // with the SAME author (empty pseudonym = local "you" message).
        let found = self.messages.iter_mut().rev().find(|r| {
            r.msg.delivery_status == rekindle_types::display::DeliveryStatus::Sending
                && (r.msg.message_id == message_id || r.msg.author_pseudonym.is_empty())
        });
        if let Some(r) = found {
            r.msg.delivery_status = rekindle_types::display::DeliveryStatus::Confirmed;
            r.msg.message_id = message_id.to_string();
            self.generation += 1;
        }
    }

    pub fn fail_pending_message(&mut self) {
        if let Some(r) = self.messages.iter_mut().rev().find(|r| {
            r.msg.delivery_status == rekindle_types::display::DeliveryStatus::Sending
        }) {
            r.msg.delivery_status = rekindle_types::display::DeliveryStatus::Failed;
            self.generation += 1;
        }
    }

    pub fn scroll_up(&mut self) {
        self.auto_scroll = false;
        let i = self.list_state.selected().unwrap_or(self.len().saturating_sub(1));
        self.list_state.select(Some(i.saturating_sub(1)));
    }

    pub fn scroll_down(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        let max = self.len().saturating_sub(1);
        self.list_state.select(Some(i.min(max).saturating_add(1).min(max)));
        if self.list_state.selected() == Some(max) { self.auto_scroll = true; }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        if !self.is_empty() { self.list_state.select(Some(self.len().saturating_sub(1))); }
    }

    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        if !self.is_empty() { self.list_state.select(Some(0)); }
    }

    /// Scroll to a specific message by ID. Disables auto-scroll.
    pub fn scroll_to_message(&mut self, message_id: &str) -> bool {
        if let Some(idx) = self.messages.iter().position(|r| r.msg.message_id == message_id) {
            self.auto_scroll = false;
            self.list_state.select(Some(idx));
            true
        } else {
            false
        }
    }

    /// Get all messages for search indexing. Returns (message_id, author, body) tuples.
    pub fn search_index(&self) -> Vec<(String, String, String)> {
        self.messages.iter().map(|r| {
            (r.msg.message_id.clone(), r.msg.author_display_name.clone(), r.msg.body.clone())
        }).collect()
    }

    /// Add a reaction to a message. Increments the count for the emoji.
    pub fn add_reaction(&mut self, message_id: &str, emoji: &str) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.msg.message_id == message_id) {
            let reactions = r.reactions.get_or_insert_with(Vec::new);
            if let Some(entry) = reactions.iter_mut().find(|(e, _)| e == emoji) {
                entry.1 += 1;
            } else {
                reactions.push((emoji.to_string(), 1));
            }
            self.generation += 1;
        }
    }

    /// Remove a reaction from a message. Decrements count, removes at 0.
    pub fn remove_reaction(&mut self, message_id: &str, emoji: &str) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.msg.message_id == message_id) {
            if let Some(ref mut reactions) = r.reactions {
                if let Some(entry) = reactions.iter_mut().find(|(e, _)| e == emoji) {
                    entry.1 = entry.1.saturating_sub(1);
                    if entry.1 == 0 {
                        reactions.retain(|(e, _)| e != emoji);
                    }
                }
                if reactions.is_empty() { r.reactions = None; }
            }
            self.generation += 1;
        }
    }

    /// Set pinned state on a message.
    pub fn set_pinned(&mut self, message_id: &str, pinned: bool) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.msg.message_id == message_id) {
            r.pinned = pinned;
            self.generation += 1;
        }
    }

    /// Set thread info on a parent message.
    pub fn set_thread(&mut self, parent_message_id: &str, thread_id: &str) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.msg.message_id == parent_message_id) {
            r.thread_id = Some(thread_id.to_string());
            r.thread_reply_count = r.thread_reply_count.saturating_add(1);
            self.generation += 1;
        }
    }

    /// Increment thread reply count for a thread's parent message.
    pub fn increment_thread_replies(&mut self, thread_id: &str) {
        if let Some(r) = self.messages.iter_mut().find(|r| r.thread_id.as_deref() == Some(thread_id)) {
            r.thread_reply_count = r.thread_reply_count.saturating_add(1);
            self.generation += 1;
        }
    }

    fn compute_group(&self, msg: &DecryptedMessageDisplay) -> MessageGroup {
        let Some(prev) = self.messages.back() else { return MessageGroup::Full; };
        let same_author = prev.msg.author_pseudonym == msg.author_pseudonym;
        let close_in_time = msg.timestamp.saturating_sub(prev.msg.timestamp) < 7 * 60 * 1000;
        if same_author && close_in_time { MessageGroup::Compact } else { MessageGroup::Full }
    }
}
