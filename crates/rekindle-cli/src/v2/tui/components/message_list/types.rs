//! Message list data types.

use rekindle_types::display::DecryptedMessageDisplay;

/// Grouping mode for a rendered message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageGroup {
    /// Full header: author name + timestamp.
    Full,
    /// Compact: no header, grouped with the message above (same author within 7 min).
    Compact,
}

/// A message with pre-computed render metadata and social annotations.
pub struct RenderedMessage {
    pub msg: DecryptedMessageDisplay,
    pub group: MessageGroup,
    /// Reaction counts: emoji → count. None when no reactions (zero allocation).
    /// Vec is used instead of HashMap because messages rarely exceed 10 unique reactions,
    /// and Vec<(String, u32)> is 24 bytes vs HashMap's 48+ bytes when empty.
    pub reactions: Option<Vec<(String, u32)>>,
    /// Whether this message is pinned.
    pub pinned: bool,
    /// Thread reply count (0 = no thread). Set when ThreadCreated references this message.
    pub thread_reply_count: u32,
    /// Thread ID if this message has a thread.
    pub thread_id: Option<String>,
    /// Cached patch fence detection — computed once on push, avoids O(n*body_len) per rebuild.
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
            thread_id: None,
            has_patch_fence: has_patch,
        }
    }
}
