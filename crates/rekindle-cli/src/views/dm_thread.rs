//! DM thread view — standalone message thread with a single peer.
//!
//! Reuses `MessageList` and `InputBox` components for feature parity
//! with `ChannelWatchView`: scroll, selection, yank, reply context,
//! typing indicator, auto-scroll on new messages.
//!
//! Layout:
//! ```text
//! ┌─ DM / alice ─────────────────────────────────────────────────┐
//! │  alice                          14:20                        │
//! │  hey, did you see the PR?                                    │
//! │                                                              │
//! │  you                            14:21                        │
//! │  yeah! the blake3 approach is smart                          │
//! │                                                              │
//! │  alice is typing...                                          │
//! ├──────────────────────────────────────────────────────────────┤
//! │  > Type a message... (i to focus)                            │
//! └──────────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use rekindle_types::display::DecryptedMessageDisplay;
use rekindle_types::subscription_events::{
    ChannelMessageEvent, SubscriptionEvent, TypingContext, TypingEvent,
};

use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::components::input_box::InputBox;
use crate::tui::components::message_list::MessageList;
use crate::tui::components::Component;
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;

use super::View;

/// Standalone DM thread view for a single peer conversation.
pub struct DmThreadView {
    /// The peer's public key.
    peer_key: String,
    /// Resolved peer display name (set from inbox data or abbreviated key).
    peer_name: String,

    /// Message list component (scroll, selection, yank, auto-scroll).
    message_list: MessageList,
    /// Message input box.
    input_box: InputBox,
    /// Focus ring: [MessageList, InputBox].
    focus: FocusRing,

    /// Whether the peer is currently typing.
    is_peer_typing: bool,
    /// Whether initial data has been loaded.
    loaded: bool,
    /// Unicode glyph support (used by components initialized from this view).
    #[allow(dead_code)]
    use_unicode: bool,
    /// Panel rects from last draw for click-to-focus hit testing.
    click_rects: std::collections::HashMap<FocusId, Rect>,
}

impl DmThreadView {
    /// Create a new DM thread view for the given peer.
    pub fn new(peer_key: String, use_unicode: bool) -> Self {
        let peer_name = helpers::abbreviate_key(&peer_key);
        Self {
            peer_key: peer_key.clone(),
            peer_name,
            message_list: MessageList::new(String::new(), peer_key),
            input_box: InputBox::new(),
            focus: FocusRing::new(vec![FocusId::MessageList, FocusId::InputBox]),
            is_peer_typing: false,
            loaded: false,
            use_unicode,
            click_rects: std::collections::HashMap::new(),
        }
    }

    /// The peer key this thread is for.
    pub fn peer_key(&self) -> &str {
        &self.peer_key
    }

    /// Build typing indicator text.
    fn typing_display(&self) -> Option<String> {
        if self.is_peer_typing {
            Some(crate::tui::components::typing_indicator::format_typing_compact(
                std::slice::from_ref(&self.peer_name),
            ))
        } else {
            None
        }
    }
}

impl View for DmThreadView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let typing_height = u16::from(self.is_peer_typing);
        let [msg_area, typing_area, input_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(typing_height),
            Constraint::Length(3),
        ])
        .areas(area);

        self.click_rects.clear();
        self.click_rects.insert(FocusId::MessageList, msg_area);
        self.click_rects.insert(FocusId::InputBox, input_area);

        self.message_list
            .set_focused(self.focus.is_focused(FocusId::MessageList));
        self.message_list.draw(frame, msg_area)?;

        if let Some(typing_text) = self.typing_display() {
            let para = ratatui::widgets::Paragraph::new(format!("  {typing_text}"))
                .style(ratatui::style::Style::new().dim().italic());
            frame.render_widget(para, typing_area);
        }

        self.input_box
            .set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area)?;

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::FocusNext => self.focus.next(),
            Action::FocusPrev => self.focus.prev(),
            Action::EnterInputMode => {
                self.focus.set(FocusId::InputBox);
            }
            Action::ExitInputMode => {
                self.focus.set(FocusId::MessageList);
            }
            Action::InputSubmit => {
                let text = self.input_box.content();
                if !text.trim().is_empty() && !self.input_box.is_over_limit() {
                    // Insert immediately with Sending status — ○ indicator
                    let now = rekindle_utils::timestamp_ms();
                    self.message_list.push(DecryptedMessageDisplay {
                        message_id: format!("pending-{now}"),
                        sequence: 0,
                        author_pseudonym: String::new(),
                        author_display_name: "you".to_string(),
                        body: text.clone(),
                        timestamp: now,
                        reply_to_sequence: None,
                        mek_generation: 0,
                        is_encrypted: false,
                        needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Sending,
                    });
                    let action = Action::SendDm {
                        peer_key: self.peer_key.clone(),
                        text,
                    };
                    self.input_box.clear();
                    return Ok(Some(action));
                }
            }
            Action::ScrollDown(n) => {
                for _ in 0..n {
                    self.message_list.scroll_down();
                }
            }
            Action::ScrollUp(n) => {
                for _ in 0..n {
                    self.message_list.scroll_up();
                }
            }
            Action::ScrollToBottom => self.message_list.scroll_to_bottom(),
            Action::ScrollToTop => self.message_list.scroll_to_top(),
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match self.focus.current() {
            FocusId::MessageList => self.message_list.handle_key(key),
            FocusId::InputBox => self.input_box.handle_key(key),
            _ => None,
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::SendFailed = result {
            self.message_list.fail_pending_message();
            return Ok(());
        }
        if let CommandResult::MessageSent { ref message_id } = result {
            // Confirm the pending DM — flip ○ to ●.
            // The pending message has a client-side nonce ("pending-{ts}") that
            // won't match the daemon's UUID. confirm_message falls back to the
            // latest Sending message when the ID doesn't match.
            self.message_list.confirm_message(message_id);
            return Ok(());
        }
        if let CommandResult::DmThreadLoaded {
            ref peer_key,
            ref messages,
        } = result
        {
            if *peer_key == self.peer_key {
                // Convert DmMessageDisplay → DecryptedMessageDisplay for MessageList
                let display_msgs: Vec<DecryptedMessageDisplay> = messages
                    .iter()
                    .enumerate()
                    .map(|(i, m)| DecryptedMessageDisplay {
                        message_id: format!("dm-{}-{i}", m.timestamp),
                        sequence: i as u64,
                        author_pseudonym: m.sender_key.clone(),
                        author_display_name: m.sender_name.clone(),
                        body: m.body.clone(),
                        timestamp: m.timestamp,
                        reply_to_sequence: None,
                        mek_generation: 0,
                        is_encrypted: false,
                        needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    })
                    .collect();
                let count = display_msgs.len();
                self.message_list.set_messages(display_msgs);
                if count > 0 {
                    self.message_list.set_last_read(count.saturating_sub(1));
                }
                // Update peer name from message data if available
                if let Some(msg) = messages.iter().find(|m| !m.is_self) {
                    self.peer_name.clone_from(&msg.sender_name);
                }
                self.loaded = true;
            }
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        match event {
            SubscriptionEvent::ChannelMessage(
                ChannelMessageEvent::DirectMessageReceived {
                    peer_key,
                    timestamp,
                    sender_name,
                    body,
                    is_self,
                },
            ) if *peer_key == self.peer_key => {
                if *is_self {
                    // Self-sent DM confirmation is handled by CommandResult::MessageSent
                    // which arrives via spawn_send_dm before this subscription event.
                    // No action needed here — the pending message is already confirmed.
                } else if let Some(ref body_text) = body {
                    // Enriched event — try to update existing placeholder
                    let msg_id = format!("dm-{timestamp}");
                    if !self.message_list.try_enrich_placeholder(
                        peer_key, *timestamp, &msg_id,
                        body_text, 0, None,
                    ) {
                        let display_name = sender_name
                            .clone()
                            .unwrap_or_else(|| helpers::abbreviate_key(peer_key));
                        self.message_list.push(DecryptedMessageDisplay {
                            message_id: msg_id,
                            sequence: 0,
                            author_pseudonym: peer_key.clone(),
                            author_display_name: display_name,
                            body: body_text.clone(),
                            timestamp: *timestamp,
                            reply_to_sequence: None,
                            mek_generation: 0,
                            is_encrypted: false,
                            needs_mek: None,
                            delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                        });
                    }
                } else {
                    // Body-less notification — show placeholder
                    let display_name = sender_name
                        .clone()
                        .unwrap_or_else(|| helpers::abbreviate_key(peer_key));
                    self.message_list.push(DecryptedMessageDisplay {
                        message_id: format!("dm-{timestamp}"),
                        sequence: 0,
                        author_pseudonym: peer_key.clone(),
                        author_display_name: display_name,
                        body: "(decrypting...)".into(),
                        timestamp: *timestamp,
                        reply_to_sequence: None,
                        mek_generation: 0,
                        is_encrypted: true,
                        needs_mek: None,
                        delivery_status: rekindle_types::display::DeliveryStatus::Confirmed,
                    });
                }
            }
            SubscriptionEvent::Typing(TypingEvent::Started {
                context: TypingContext::Dm { peer_key },
                ..
            }) if *peer_key == self.peer_key => {
                self.is_peer_typing = true;
            }
            SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Dm { peer_key },
                ..
            }) if *peer_key == self.peer_key => {
                self.is_peer_typing = false;
            }
            _ => {}
        }
        Ok(())
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

    fn typing_names(&self) -> Vec<String> {
        if self.is_peer_typing {
            vec![self.peer_name.clone()] // Single-element vec for status bar display
        } else {
            Vec::new()
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
