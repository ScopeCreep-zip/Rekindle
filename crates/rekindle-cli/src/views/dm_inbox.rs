//! DM inbox view — conversation list and thread display.
//!
//! Two-pane layout:
//! - Left: DM conversations sorted by most recent, with unread counts
//! - Right: message thread for the selected conversation + input box
//!
//! Layout:
//! ```text
//! ┌─ Conversations ──────┬─ Messages ───────────────────────┐
//! │  ● alice (2 new)     │  [14:31] alice: Hey              │
//! │  ○ bob               │  [14:32] you: What's up?         │
//! │  ● carol (1 new)     │                                  │
//! │                      ├──────────────────────────────────┤
//! │                      │  [INSERT] type...                │
//! └──────────────────────┴──────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::display::{DmMessageDisplay, DmThreadDisplay};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use super::View;
use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::components::input_box::InputBox;
use crate::tui::components::Component;
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;

/// DM inbox view state.
pub struct DmInboxView {
    /// Focus ring for the two panes + input.
    focus: FocusRing,
    /// Conversation list (sorted by most recent).
    threads: Vec<DmThreadDisplay>,
    /// Conversation list selection state.
    thread_list_state: ListState,
    /// Input box for the selected conversation.
    input_box: InputBox,
    /// Whether we have loaded data yet.
    loaded: bool,
    /// Unicode glyph support.
    use_unicode: bool,
    /// Per-conversation scroll positions: peer_key → (list_selection_index).
    /// Saved when switching away from a thread, restored when switching back.
    scroll_positions: std::collections::HashMap<String, usize>,
    /// Panel rects from last draw for click-to-focus.
    click_rects: std::collections::HashMap<FocusId, Rect>,
}

impl DmInboxView {
    /// Create a new DM inbox view.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![
                FocusId::DmList,
                FocusId::MessageList,
                FocusId::InputBox,
            ]),
            threads: Vec::new(),
            thread_list_state: ListState::default(),
            input_box: InputBox::new(),
            loaded: false,
            use_unicode,
            scroll_positions: std::collections::HashMap::new(),
            click_rects: std::collections::HashMap::new(),
        }
    }

    /// Save the current thread's scroll position before switching.
    fn save_scroll_position(&mut self) {
        if let Some(idx) = self.thread_list_state.selected() {
            if let Some(thread) = self.threads.get(idx) {
                // Save the thread list selection index for this peer
                self.scroll_positions.insert(thread.peer_key.clone(), idx);
            }
        }
    }

    /// Restore a thread's scroll position when switching back.
    fn restore_scroll_position(&mut self, peer_key: &str) {
        if let Some(&saved_idx) = self.scroll_positions.get(peer_key) {
            let max = self.threads.len().saturating_sub(1);
            self.thread_list_state.select(Some(saved_idx.min(max)));
        }
    }

    /// The currently selected thread's peer key, if any.
    fn selected_peer_key(&self) -> Option<&str> {
        self.thread_list_state
            .selected()
            .and_then(|i| self.threads.get(i))
            .map(|t| t.peer_key.as_str())
    }

    /// Build conversation list items.
    fn build_thread_items(&self) -> Vec<ListItem<'static>> {
        self.threads
            .iter()
            .map(|thread| {
                let name = helpers::sanitize_for_display(&thread.peer_name);
                let unread = thread.messages.iter().filter(|m| !m.is_self).count();
                let time = if thread.last_message_at > 0 {
                    helpers::format_time_short(thread.last_message_at)
                } else {
                    String::new()
                };

                let unread_badge = if unread > 0 {
                    format!(" ({unread})")
                } else {
                    String::new()
                };

                let glyph = if unread > 0 {
                    if self.use_unicode {
                        "● "
                    } else {
                        "* "
                    }
                } else {
                    "  "
                };

                let line = Line::from(vec![
                    Span::raw(format!("  {glyph}")),
                    Span::styled(name, Style::new().bold()),
                    Span::styled(unread_badge, Style::new().bold()),
                    Span::styled(format!("  {time}"), Style::new().dim()),
                ]);
                ListItem::new(line)
            })
            .collect()
    }

    /// Render the message thread for the selected conversation.
    fn render_thread(&self, frame: &mut Frame, area: Rect, thread: &DmThreadDisplay) {
        let block = Block::bordered()
            .title(format!(
                " {} ",
                helpers::sanitize_for_display(&thread.peer_name)
            ))
            .border_style(if self.focus.is_focused(FocusId::MessageList) {
                Style::new()
            } else {
                Style::new().dim()
            });

        if thread.messages.is_empty() {
            let para = Paragraph::new("  No messages yet.")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return;
        }

        let lines: Vec<Line<'_>> = thread
            .messages
            .iter()
            .map(|msg| {
                let sender = if msg.is_self { "you" } else { &msg.sender_name };
                let sender = helpers::sanitize_for_display(sender);
                let body = helpers::sanitize_for_display(&msg.body);
                Line::from(vec![
                    Span::styled(
                        format!("  [{}] ", helpers::format_time_short(msg.timestamp)),
                        Style::new().dim(),
                    ),
                    Span::styled(format!("{sender}: "), Style::new().bold()),
                    Span::raw(body),
                ])
            })
            .collect();

        let para = Paragraph::new(lines).block(block);
        frame.render_widget(para, area);
    }
}

impl View for DmInboxView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        // Two-pane layout: conversation list | thread + input
        let [list_area, thread_area] =
            Layout::horizontal([Constraint::Length(28), Constraint::Fill(1)]).areas(area);

        self.click_rects.clear();
        self.click_rects.insert(FocusId::DmList, list_area);

        // Conversation list
        let list_block = Block::bordered()
            .title(format!(" DMs ({}) ", self.threads.len()))
            .border_style(if self.focus.is_focused(FocusId::DmList) {
                Style::new()
            } else {
                Style::new().dim()
            });

        if self.threads.is_empty() {
            let para = Paragraph::new("  No conversations.")
                .style(Style::new().dim())
                .block(list_block);
            frame.render_widget(para, list_area);
        } else {
            let items = self.build_thread_items();
            let list = List::new(items)
                .block(list_block)
                .highlight_style(Style::new().reversed());
            frame.render_stateful_widget(list, list_area, &mut self.thread_list_state);
        }

        // Thread + input
        let [msg_area, input_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(thread_area);

        // Render selected thread
        if let Some(idx) = self.thread_list_state.selected() {
            if let Some(thread) = self.threads.get(idx) {
                self.render_thread(frame, msg_area, thread);
            }
        } else {
            let block = Block::bordered()
                .title(" Messages ")
                .border_style(Style::new().dim());
            let para = Paragraph::new("  Select a conversation.")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, msg_area);
        }

        self.click_rects.insert(FocusId::MessageList, msg_area);

        // Input box
        self.input_box
            .set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area)?;
        self.click_rects.insert(FocusId::InputBox, input_area);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::FocusNext => self.focus.next(),
            Action::FocusPrev => self.focus.prev(),
            Action::ScrollDown(_) if self.focus.is_focused(FocusId::DmList) => {
                self.save_scroll_position();
                let max = self.threads.len().saturating_sub(1);
                let i = self.thread_list_state.selected().unwrap_or(0);
                self.thread_list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) if self.focus.is_focused(FocusId::DmList) => {
                self.save_scroll_position();
                let i = self.thread_list_state.selected().unwrap_or(0);
                self.thread_list_state.select(Some(i.saturating_sub(1)));
            }
            Action::Select if self.focus.is_focused(FocusId::DmList) => {
                // Save position before switching, restore target's position
                self.save_scroll_position();
                if let Some(peer_key) = self.selected_peer_key().map(str::to_string) {
                    self.restore_scroll_position(&peer_key);
                }
                self.focus.set(FocusId::MessageList);
            }
            Action::InputSubmit => {
                let text = self.input_box.content();
                if let Some(peer_key) = self.selected_peer_key() {
                    if !text.trim().is_empty() && !self.input_box.is_over_limit() {
                        let action = Action::SendDm {
                            peer_key: peer_key.to_string(),
                            text,
                        };
                        self.input_box.clear();
                        return Ok(Some(action));
                    }
                }
            }
            Action::EnterInputMode => {
                self.focus.set(FocusId::InputBox);
            }
            Action::ExitInputMode => {
                self.focus.set(FocusId::DmList);
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match self.focus.current() {
            FocusId::InputBox => self.input_box.handle_key(key),
            _ => None,
        }
    }

    fn handle_click(&mut self, column: u16, row: u16) -> Option<Action> {
        for (&id, rect) in &self.click_rects {
            if column >= rect.x
                && column < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height
            {
                self.focus.set(id);
                if id == FocusId::InputBox {
                    return Some(Action::EnterInputMode);
                }
                return None;
            }
        }
        None
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        if let SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key,
            timestamp,
            sender_name,
            body,
        }) = event
        {
            let display_msg = DmMessageDisplay {
                sender_key: peer_key.clone(),
                sender_name: sender_name
                    .clone()
                    .unwrap_or_else(|| helpers::abbreviate_key(peer_key)),
                body: body.clone().unwrap_or_else(|| "(decrypting...)".into()),
                timestamp: *timestamp,
                is_self: false,
            };

            if let Some(thread) = self.threads.iter_mut().find(|t| t.peer_key == *peer_key) {
                thread.messages.push(display_msg);
                thread.last_message_at = *timestamp;
                thread.unread_count += 1;
            } else {
                self.threads.push(DmThreadDisplay {
                    peer_key: peer_key.clone(),
                    peer_name: sender_name
                        .clone()
                        .unwrap_or_else(|| helpers::abbreviate_key(peer_key)),
                    last_message_at: *timestamp,
                    unread_count: 1,
                    messages: vec![display_msg],
                });
            }

            self.threads
                .sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
        }
        Ok(())
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::DmInboxLoaded { threads } = result {
            self.threads = threads;
            self.threads
                .sort_by(|a, b| b.last_message_at.cmp(&a.last_message_at));
            if !self.threads.is_empty() && self.thread_list_state.selected().is_none() {
                self.thread_list_state.select(Some(0));
            }
            self.loaded = true;
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
