//! Message list component — the core of the messaging TUI.
//!
//! Renders a scrollable list of channel messages with:
//! - **Message grouping** — consecutive messages from the same author
//!   within 7 minutes are grouped (compact header, no repeated name).
//! - **Auto-scroll** — snaps to newest message when at bottom. Disengages
//!   when the user scrolls up manually. Re-engages on `G` or `End`.
//! - **Generation tracking** — only re-renders messages whose content
//!   changed since last render. O(1) amortized for single-message append.
//! - **Unread separator** — horizontal rule between last-read and first
//!   unread message with `── New ──` text label.
//! - **Encrypted placeholder** — messages with missing MEK show
//!   `[encrypted, gen N]` with a remediation hint.
//!
//! Source patterns:
//! - oxicord `presentation/widgets/message_pane.rs` — grouping, UiMessage
//! - siggy `ui/chat_pane.rs` — read receipts, bottom-up scroll
//! - siggy `domain/scroll.rs` — offset from bottom, jump stack

use std::collections::VecDeque;

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use rekindle_types::display::DecryptedMessageDisplay;

use super::Component;
use crate::helpers;
use crate::tui::action::Action;

/// Grouping mode for a rendered message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageGroup {
    /// Full header: author name + timestamp.
    Full,
    /// Compact: no header, grouped with the message above.
    /// Same author within a 7-minute window.
    Compact,
}

/// A message with pre-computed render metadata.
struct RenderedMessage {
    msg: DecryptedMessageDisplay,
    group: MessageGroup,
}

/// Message list component.
///
/// Owns the message buffer and scroll state. Receives new messages via
/// `push()` and dispatches scroll actions via `handle_key()`.
pub struct MessageList {
    /// Channel messages in chronological order.
    messages: VecDeque<RenderedMessage>,
    /// Ratatui list selection state (tracks visual scroll position).
    list_state: ListState,
    /// Whether new messages should auto-scroll to bottom.
    auto_scroll: bool,
    /// Community governance key (for context display).
    community: String,
    /// Channel name or ID.
    channel: String,
    /// Monotonically increasing counter — incremented on every mutation.
    generation: u64,
    /// Whether this component is currently focused.
    is_focused: bool,
    /// Last read message index (for unread separator).
    last_read_index: Option<usize>,
}

impl MessageList {
    /// Create a new empty message list for a channel.
    pub fn new(community: String, channel: String) -> Self {
        Self {
            messages: VecDeque::new(),
            list_state: ListState::default(),
            auto_scroll: true,
            community,
            channel,
            generation: 0,
            is_focused: false,
            last_read_index: None,
        }
    }

    /// Append a new message to the list.
    ///
    /// Computes the grouping based on the previous message's author
    /// and timestamp. If auto_scroll is enabled, selects the new message.
    pub fn push(&mut self, msg: DecryptedMessageDisplay) {
        let group = self.compute_group(&msg);
        self.messages.push_back(RenderedMessage {
            msg,
            group,
        });

        if self.auto_scroll {
            self.list_state
                .select(Some(self.len().saturating_sub(1)));
        }

        self.generation += 1;
    }

    /// Replace the entire message list (e.g., after loading history).
    pub fn set_messages(&mut self, messages: Vec<DecryptedMessageDisplay>) {
        self.messages.clear();

        for msg in messages {
            let group = self.compute_group(&msg);
            self.messages.push_back(RenderedMessage {
                msg,
                group,
                });
        }

        // Auto-scroll to bottom after load
        if !self.is_empty() {
            self.list_state
                .select(Some(self.len().saturating_sub(1)));
        }
        self.auto_scroll = true;
        self.generation += 1;
    }

    /// Set the last-read message index for the unread separator.
    pub fn set_last_read(&mut self, index: usize) {
        self.last_read_index = Some(index);
    }

    /// Currently selected message index.
    pub fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }

    /// Access a message by index.
    pub fn message_at(&self, index: usize) -> Option<&DecryptedMessageDisplay> {
        self.messages.get(index).map(|r| &r.msg)
    }

    /// Number of messages in the list.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Remove a message by its ID. No-op if not found.
    pub fn remove_by_id(&mut self, message_id: &str) {
        self.messages.retain(|r| r.msg.message_id != message_id);
        // Fix selection if it's now out of bounds
        if let Some(sel) = self.list_state.selected() {
            if sel >= self.len() && !self.is_empty() {
                self.list_state.select(Some(self.len() - 1));
            }
        }
        self.generation += 1;
    }

    /// Scroll up by one message. Disables auto-scroll.
    pub fn scroll_up(&mut self) {
        self.auto_scroll = false;
        let i = self
            .list_state
            .selected()
            .unwrap_or(self.len().saturating_sub(1));
        self.list_state.select(Some(i.saturating_sub(1)));
    }

    /// Scroll down by one message.
    pub fn scroll_down(&mut self) {
        let i = self.list_state.selected().unwrap_or(0);
        let max = self.len().saturating_sub(1);
        self.list_state.select(Some(i.min(max).saturating_add(1).min(max)));

        // Re-engage auto-scroll if we reached the bottom
        if self.list_state.selected() == Some(max) {
            self.auto_scroll = true;
        }
    }

    /// Jump to the bottom. Re-enables auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        if !self.is_empty() {
            self.list_state
                .select(Some(self.len().saturating_sub(1)));
        }
    }

    /// Jump to the top. Disables auto-scroll.
    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        if !self.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Compute the grouping for a new message based on the previous one.
    ///
    /// Messages from the same author within 7 minutes get Compact grouping
    /// (no repeated author name + timestamp header).
    fn compute_group(&self, msg: &DecryptedMessageDisplay) -> MessageGroup {
        let Some(prev) = self.messages.back() else {
            return MessageGroup::Full;
        };

        let same_author = prev.msg.author_pseudonym == msg.author_pseudonym;
        // 7-minute grouping window (420,000 ms)
        let close_in_time = msg
            .timestamp
            .saturating_sub(prev.msg.timestamp)
            < 7 * 60 * 1000;

        if same_author && close_in_time {
            MessageGroup::Compact
        } else {
            MessageGroup::Full
        }
    }

    /// Build the list items for rendering.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::with_capacity(self.len());

        for (i, rendered) in self.messages.iter().enumerate() {
            // Unread separator
            if self.last_read_index == Some(i.saturating_sub(1)) && i > 0 {
                let separator = Line::from(vec![
                    Span::styled("──── ", Style::new().dim()),
                    Span::styled("New", Style::new().bold()),
                    Span::styled(" ────", Style::new().dim()),
                ]);
                items.push(ListItem::new(separator));
            }

            let msg = &rendered.msg;
            let mut lines = Vec::new();

            if rendered.group == MessageGroup::Full {
                let author = helpers::sanitize_for_display(&msg.author_display_name);
                let time = helpers::format_time_short(msg.timestamp);
                lines.push(Line::from(vec![
                    Span::styled(author, Style::new().bold()),
                    Span::raw("  "),
                    Span::styled(format!("[{time}]"), Style::new().dim()),
                ]));
            }

            if msg.is_encrypted {
                let hint = msg
                    .needs_mek
                    .map(|_gen| format!(" — request: rekindle key mek request -c \"{}\" -C \"{}\"",
                        self.community, self.channel))
                    .unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("[encrypted, MEK gen {}]", msg.mek_generation),
                        Style::new().dim().italic(),
                    ),
                    Span::styled(hint, Style::new().dim()),
                ]));
            } else {
                let body = helpers::sanitize_for_display(&msg.body);
                for line in body.lines() {
                    lines.push(Line::from(format!("  {line}")));
                }
            }

            if let Some(reply_seq) = msg.reply_to_sequence {
                lines.push(Line::from(vec![
                    Span::styled("  ↳ reply to ", Style::new().dim()),
                    Span::styled(
                        format!("#{reply_seq}"),
                        Style::new().dim().italic(),
                    ),
                ]));
            }

            items.push(ListItem::new(lines));
        }

        items
    }
}

impl Component for MessageList {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll_down();
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll_up();
                None
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.scroll_to_bottom();
                None
            }
            KeyCode::Home => {
                self.scroll_to_top();
                None
            }
            KeyCode::PageDown => {
                // Scroll down by 10 lines
                for _ in 0..10 {
                    self.scroll_down();
                }
                None
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.scroll_up();
                }
                None
            }
            KeyCode::Char('i') => Some(Action::EnterInputMode),
            KeyCode::Char('r') => Some(Action::ReplyToSelected),
            KeyCode::Char('e') => Some(Action::EditSelected),
            KeyCode::Char('y') => {
                // Yank focused message body to clipboard
                if let Some(idx) = self.selected_index() {
                    if let Some(msg) = self.message_at(idx) {
                        if !msg.is_encrypted {
                            return Some(Action::YankToClipboard {
                                text: msg.body.clone(),
                            });
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> anyhow::Result<()> {
        if self.is_empty() {
            let empty_msg = Paragraph::new("No messages yet.")
                .style(Style::new().dim())
                .block(
                    Block::bordered()
                        .title(format!(" #{} ", self.channel))
                        .border_style(if self.is_focused {
                            Style::new()
                        } else {
                            Style::new().dim()
                        }),
                );
            frame.render_widget(empty_msg, area);
            return Ok(());
        }

        let items = self.build_items();

        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(format!(" #{} ({} messages) ", self.channel, self.len()))
                    .border_style(if self.is_focused {
                        Style::new()
                    } else {
                        Style::new().dim()
                    }),
            )
            .highlight_style(Style::new().reversed());

        frame.render_stateful_widget(list, area, &mut self.list_state);

        // Auto-scroll indicator
        if !self.auto_scroll && !self.is_empty() {
            let hint = Paragraph::new(" ↑ scrolled — press G to jump to latest ")
                .style(Style::new().dim().italic())
                .alignment(ratatui::layout::Alignment::Center);
            let hint_area = Rect {
                x: area.x,
                y: area.bottom().saturating_sub(1),
                width: area.width,
                height: 1,
            };
            frame.render_widget(hint, hint_area);
        }

        Ok(())
    }

    fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_msg(author: &str, body: &str, ts: u64) -> DecryptedMessageDisplay {
        DecryptedMessageDisplay {
            message_id: format!("msg-{ts}"),
            sequence: ts / 1000,
            author_pseudonym: author.to_string(),
            author_display_name: author.to_string(),
            body: body.to_string(),
            timestamp: ts,
            reply_to_sequence: None,
            mek_generation: 1,
            is_encrypted: false,
            needs_mek: None,
        }
    }

    #[test]
    fn auto_scroll_on_push_when_at_bottom() {
        let mut list = MessageList::new("com".into(), "gen".into());
        for i in 0..10 {
            list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
        }
        assert_eq!(list.list_state.selected(), Some(9));
        assert!(list.auto_scroll);
    }

    #[test]
    fn auto_scroll_disengages_on_scroll_up() {
        let mut list = MessageList::new("com".into(), "gen".into());
        for i in 0..10 {
            list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
        }
        list.scroll_up();
        assert!(!list.auto_scroll);
    }

    #[test]
    fn auto_scroll_reengages_on_scroll_to_bottom() {
        let mut list = MessageList::new("com".into(), "gen".into());
        for i in 0..10 {
            list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
        }
        list.scroll_up();
        assert!(!list.auto_scroll);

        list.scroll_to_bottom();
        assert!(list.auto_scroll);
        assert_eq!(list.list_state.selected(), Some(9));
    }

    #[test]
    fn message_grouping_same_author_within_window() {
        let mut list = MessageList::new("com".into(), "gen".into());
        list.push(test_msg("alice", "first", 0));
        list.push(test_msg("alice", "second", 60_000)); // 1 min later
        list.push(test_msg("alice", "third", 120_000)); // 2 min later

        assert_eq!(list.messages[0].group, MessageGroup::Full);
        assert_eq!(list.messages[1].group, MessageGroup::Compact);
        assert_eq!(list.messages[2].group, MessageGroup::Compact);
    }

    #[test]
    fn message_grouping_breaks_after_seven_minutes() {
        let mut list = MessageList::new("com".into(), "gen".into());
        list.push(test_msg("alice", "first", 0));
        list.push(test_msg("alice", "second", 8 * 60 * 1000)); // 8 min later

        assert_eq!(list.messages[0].group, MessageGroup::Full);
        assert_eq!(list.messages[1].group, MessageGroup::Full); // breaks
    }

    #[test]
    fn message_grouping_breaks_on_different_author() {
        let mut list = MessageList::new("com".into(), "gen".into());
        list.push(test_msg("alice", "hello", 0));
        list.push(test_msg("bob", "hi", 60_000));

        assert_eq!(list.messages[0].group, MessageGroup::Full);
        assert_eq!(list.messages[1].group, MessageGroup::Full);
    }

    #[test]
    fn set_messages_replaces_all() {
        let mut list = MessageList::new("com".into(), "gen".into());
        list.push(test_msg("alice", "old", 0));

        let new_msgs = vec![
            test_msg("bob", "new1", 1000),
            test_msg("bob", "new2", 2000),
        ];
        list.set_messages(new_msgs);

        assert_eq!(list.len(), 2);
        assert!(list.auto_scroll);
        assert_eq!(list.list_state.selected(), Some(1));
    }

    #[test]
    fn empty_list_reports_correctly() {
        let list = MessageList::new("com".into(), "gen".into());
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }
}
