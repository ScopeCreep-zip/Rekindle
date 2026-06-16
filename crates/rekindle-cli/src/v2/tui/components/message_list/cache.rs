//! Message render data — grouping, reactions, delivery tracking, placeholder enrichment.

use std::collections::{HashMap, VecDeque};

use rekindle_types::display::{DecryptedMessageDisplay, DeliveryStatus, Groupable};

use crate::v2::tui::state::dm::DmMessage;

/// Grouping mode for a rendered message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MessageGroup {
    /// Full header: author name + timestamp.
    Full,
    /// No header, grouped with the message above. Same author within 7 minutes.
    Compact,
}

/// Pre-computed render metadata for one message.
pub struct RenderedMessage {
    pub msg: DecryptedMessageDisplay,
    pub group: MessageGroup,
    /// Emoji → count. None when no reactions (zero allocation).
    pub reactions: Option<Vec<(String, u32)>>,
    pub pinned: bool,
    /// 0 = no thread.
    pub thread_reply_count: u32,
    /// Cached detection — avoids O(n * body_len) per rebuild.
    pub has_patch_fence: bool,
}

impl RenderedMessage {
    pub fn new(msg: DecryptedMessageDisplay, group: MessageGroup) -> Self {
        let has_patch = crate::v2::patch::render::extract_patch_fence(&msg.body).is_some();
        Self {
            msg,
            group,
            reactions: None,
            pinned: false,
            thread_reply_count: 0,
            has_patch_fence: has_patch,
        }
    }

    pub fn from_dm(
        dm: &rekindle_types::display::DmMessageDisplay,
        delivery_status: DeliveryStatus,
        group: MessageGroup,
    ) -> Self {
        let has_patch = crate::v2::patch::render::extract_patch_fence(&dm.body).is_some();
        Self {
            msg: DecryptedMessageDisplay {
                message_id: String::new(),
                sequence: dm.sequence,
                author_pseudonym: dm.sender_key.clone(),
                author_display_name: dm.sender_name.clone(),
                body: dm.body.clone(),
                timestamp: dm.timestamp,
                reply_to_sequence: None,
                mek_generation: 0,
                is_encrypted: false,
                needs_mek: None,
                delivery_status,
                thread_id: None,
            },
            group,
            reactions: None,
            pinned: false,
            thread_reply_count: 0,
            has_patch_fence: has_patch,
        }
    }
}

/// Compute grouping for a message given the previous message.
/// Same author within 7 minutes = Compact, otherwise Full.
pub fn compute_group<T: Groupable>(prev: Option<&T>, msg: &T) -> MessageGroup {
    let Some(prev) = prev else {
        return MessageGroup::Full;
    };
    let same_author = prev.author_id() == msg.author_id();
    let close_in_time = msg.message_timestamp().saturating_sub(prev.message_timestamp()) < 7 * 60 * 1000;
    if same_author && close_in_time {
        MessageGroup::Compact
    } else {
        MessageGroup::Full
    }
}

/// Build RenderedMessage list from channel messages, enriched with reactions.
pub fn build_rendered_channel(
    messages: &VecDeque<DecryptedMessageDisplay>,
    reactions: &HashMap<String, Vec<(String, u32)>>,
) -> Vec<RenderedMessage> {
    let mut result = Vec::with_capacity(messages.len());
    for (i, msg) in messages.iter().enumerate() {
        let prev = if i > 0 { messages.get(i - 1) } else { None };
        let group = compute_group(prev, msg);
        let mut rm = RenderedMessage::new(msg.clone(), group);
        if let Some(msg_reactions) = reactions.get(&msg.message_id) {
            if !msg_reactions.is_empty() {
                rm.reactions = Some(msg_reactions.clone());
            }
        }
        result.push(rm);
    }
    result
}

/// Build RenderedMessage list from DM messages.
pub fn build_rendered_dm(messages: &VecDeque<DmMessage>) -> Vec<RenderedMessage> {
    let mut result = Vec::with_capacity(messages.len());
    for (i, dm) in messages.iter().enumerate() {
        let prev = if i > 0 { Some(&messages[i - 1].display) } else { None };
        let group = compute_group(prev, &dm.display);
        result.push(RenderedMessage::from_dm(&dm.display, dm.delivery_status, group));
    }
    result
}

