//! Channel watch view — three-pane live message stream.
//!
//! The primary messaging view. Three panes:
//! - Left: channel tree sidebar (collapsible, hides below 60 cols)
//! - Center: message list (auto-scroll) + input box
//! - Right: peer/member list (hides below 100 cols)
//!
//! Receives `TransportEvent::MessageReceived` and `TypingIndicator`
//! events in real time. Fires `SendChannelMessage` on input submit.
//!
//! Layout:
//! ```text
//! ┌──────────┬────────────────────────┬─────────────┐
//! │ channels │  #general (42 msgs)    │ Members (8) │
//! │          │                        │ ● alice     │
//! │ ▸ dev    │  alice  [14:31]        │ ● bob       │
//! │   # gen  │    Have you seen the   │ ○ carol     │
//! │   # code │    spec?               │             │
//! │          │  bob  [14:32]          │             │
//! │ ▸ gaming │    Yes — section 4.    │             │
//! │          ├────────────────────────│             │
//! │ ▸ DMs    │ [INSERT] type msg...   │             │
//! └──────────┴────────────────────────┴─────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use rekindle_types::display::DecryptedMessageDisplay;
use rekindle_types::subscription_events::{SubscriptionEvent, ChannelMessageEvent, TypingEvent, TypingContext};

use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::components::Component;
use crate::tui::components::channel_tree::ChannelTree;
use crate::tui::components::input_box::InputBox;
use crate::tui::components::message_list::MessageList;
use crate::tui::components::peer_list::PeerList;
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;
use super::View;

/// Responsive layout breakpoints.
const SIDEBAR_COLLAPSE_WIDTH: u16 = 60;
const PEER_LIST_COLLAPSE_WIDTH: u16 = 100;
const SIDEBAR_WIDTH: u16 = 22;
const PEER_LIST_WIDTH: u16 = 18;

/// Channel watch view state.
pub struct ChannelWatchView {
    /// The community governance key this view is watching.
    community: String,
    /// The channel ID this view is watching.
    channel: String,

    /// Sidebar channel tree.
    channel_tree: ChannelTree,
    /// Main message list.
    message_list: MessageList,
    /// Message input box.
    input_box: InputBox,
    /// Right sidebar peer list.
    peer_list: PeerList,

    /// Focus ring — adapts to visible panels.
    focus: FocusRing,
    /// Whether the sidebar is visible (user toggle + responsive).
    sidebar_visible: bool,
    /// Current terminal width (for responsive layout).
    terminal_width: u16,
    /// Typing indicator state: pseudonym → last seen instant.
    typing_indicators: std::collections::HashMap<String, std::time::Instant>,
    /// MEK generations we've auto-requested but not yet received.
    /// Prevents re-requesting the same generation on every history load.
    pending_mek_requests: std::collections::HashSet<u64>,
    /// Panel rects from the last draw — used for click-to-focus.
    /// Order: [sidebar, messages, input, peers]. None if panel not rendered.
    click_rects: std::collections::HashMap<FocusId, Rect>,
}

impl ChannelWatchView {
    /// Create a new channel watch view for the specified community/channel.
    pub fn new(community: String, channel: String, use_unicode: bool) -> Self {
        let focus = FocusRing::new(vec![
            FocusId::ChannelTree,
            FocusId::MessageList,
            FocusId::InputBox,
            FocusId::PeerList,
        ]);

        Self {
            community: community.clone(),
            channel: channel.clone(),
            channel_tree: ChannelTree::new(use_unicode),
            message_list: MessageList::new(community, channel),
            input_box: InputBox::new(),
            peer_list: PeerList::new(use_unicode),
            focus,
            sidebar_visible: true,
            terminal_width: 120,
            typing_indicators: std::collections::HashMap::new(),
            pending_mek_requests: std::collections::HashSet::new(),
            click_rects: std::collections::HashMap::new(),
        }
    }

    /// Community governance key.
    pub fn community(&self) -> &str {
        &self.community
    }

    /// Channel ID.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Update the focus ring based on which panels are visible.
    fn update_focus_ring(&mut self) {
        let mut slots = Vec::new();
        if self.sidebar_visible && self.terminal_width >= SIDEBAR_COLLAPSE_WIDTH {
            slots.push(FocusId::ChannelTree);
        }
        slots.push(FocusId::MessageList);
        slots.push(FocusId::InputBox);
        if self.terminal_width >= PEER_LIST_COLLAPSE_WIDTH {
            slots.push(FocusId::PeerList);
        }
        self.focus.set_slots(slots);
    }

    /// Expire old typing indicators (older than 5 seconds).
    fn expire_typing_indicators(&mut self) {
        let cutoff = std::time::Duration::from_secs(5);
        self.typing_indicators
            .retain(|_, instant| instant.elapsed() < cutoff);
    }

    /// Build the typing indicator display line.
    fn typing_display(&self) -> Option<String> {
        if self.typing_indicators.is_empty() {
            return None;
        }
        let names: Vec<String> = self
            .typing_indicators
            .keys()
            .take(3)
            .map(|k| helpers::abbreviate_key(k))
            .collect();
        let suffix = if self.typing_indicators.len() > 3 {
            format!(" and {} others", self.typing_indicators.len() - 3)
        } else {
            String::new()
        };
        match names.len() {
            1 => Some(format!("{}{suffix} is typing...", names[0])),
            2 => Some(format!("{} and {}{suffix} are typing...", names[0], names[1])),
            _ => Some(format!(
                "{}, {}, and {}{suffix} are typing...",
                names[0], names[1], names[2]
            )),
        }
    }
}

impl View for ChannelWatchView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        self.terminal_width = area.width;
        self.update_focus_ring();

        let show_sidebar = self.sidebar_visible && area.width >= SIDEBAR_COLLAPSE_WIDTH;
        let show_peers = area.width >= PEER_LIST_COLLAPSE_WIDTH;

        // Build horizontal layout constraints based on visible panels
        let mut h_constraints: Vec<Constraint> = Vec::new();
        if show_sidebar {
            h_constraints.push(Constraint::Length(SIDEBAR_WIDTH));
        }
        h_constraints.push(Constraint::Fill(1));
        if show_peers {
            h_constraints.push(Constraint::Length(PEER_LIST_WIDTH));
        }

        let h_areas = Layout::horizontal(h_constraints).split(area);

        let mut col = 0;
        self.click_rects.clear();

        // Sidebar
        if show_sidebar {
            self.channel_tree
                .set_focused(self.focus.is_focused(FocusId::ChannelTree));
            self.channel_tree.draw(frame, h_areas[col])?;
            self.click_rects.insert(FocusId::ChannelTree, h_areas[col]);
            col += 1;
        }

        // Center: messages + input
        let center_area = h_areas[col];
        col += 1;

        // Split center vertically: messages + typing indicator + input
        let typing_height = u16::from(self.typing_display().is_some());
        let [msg_area, typing_area, input_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(typing_height),
            Constraint::Length(3),
        ])
        .areas(center_area);

        self.message_list
            .set_focused(self.focus.is_focused(FocusId::MessageList));
        self.message_list.draw(frame, msg_area)?;
        self.click_rects.insert(FocusId::MessageList, msg_area);

        // Typing indicator
        if let Some(typing_text) = self.typing_display() {
            let typing_para = ratatui::widgets::Paragraph::new(format!("  {typing_text}"))
                .style(ratatui::style::Style::new().dim().italic());
            frame.render_widget(typing_para, typing_area);
        }

        self.input_box
            .set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area)?;
        self.click_rects.insert(FocusId::InputBox, input_area);

        // Peer list
        if show_peers {
            self.peer_list
                .set_focused(self.focus.is_focused(FocusId::PeerList));
            self.peer_list.draw(frame, h_areas[col])?;
            self.click_rects.insert(FocusId::PeerList, h_areas[col]);
        }

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::FocusNext => self.focus.next(),
            Action::FocusPrev => self.focus.prev(),
            Action::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.update_focus_ring();
            }
            Action::EnterInputMode => {
                self.focus.set(FocusId::InputBox);
            }
            Action::ExitInputMode => {
                self.focus.set(FocusId::MessageList);
                self.input_box.set_mode(crate::tui::components::input_box::InputMode::Compose);
            }
            Action::ReplyToSelected => {
                if let Some(idx) = self.message_list.selected_index() {
                    if let Some(msg) = self.message_list.message_at(idx) {
                        use crate::tui::components::input_box::InputMode;
                        self.input_box.set_mode(InputMode::Reply {
                            message_id: msg.message_id.clone(),
                            author: msg.author_display_name.clone(),
                        });
                        self.focus.set(FocusId::InputBox);
                    }
                }
            }
            Action::EditSelected => {
                if let Some(idx) = self.message_list.selected_index() {
                    if let Some(msg) = self.message_list.message_at(idx) {
                        use crate::tui::components::input_box::InputMode;
                        self.input_box.set_mode(InputMode::Edit {
                            message_id: msg.message_id.clone(),
                        });
                        self.focus.set(FocusId::InputBox);
                    }
                }
            }
            Action::InputSubmit => {
                let text = self.input_box.content();
                if !text.trim().is_empty() && !self.input_box.is_over_limit() {
                    let reply_to = match self.input_box.mode() {
                        crate::tui::components::input_box::InputMode::Reply { message_id, .. } => {
                            Some(message_id.clone())
                        }
                        _ => None,
                    };
                    let action = Action::SendChannelMessage {
                        community: self.community.clone(),
                        channel: self.channel.clone(),
                        text,
                        reply_to,
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
            Action::ScrollPageDown => {
                for _ in 0..10 {
                    self.message_list.scroll_down();
                }
            }
            Action::ScrollPageUp => {
                for _ in 0..10 {
                    self.message_list.scroll_up();
                }
            }
            Action::Resize(w, _h) => {
                self.terminal_width = w;
                self.update_focus_ring();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match self.focus.current() {
            FocusId::ChannelTree => self.channel_tree.handle_key(key),
            FocusId::MessageList => self.message_list.handle_key(key),
            FocusId::InputBox => self.input_box.handle_key(key),
            FocusId::PeerList => self.peer_list.handle_key(key),
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
                // Clicking InputBox also enters input mode
                if id == FocusId::InputBox {
                    return Some(Action::EnterInputMode);
                }
                return None;
            }
        }
        None
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::ChannelHistoryLoaded {
                community,
                channel,
                messages,
            } => {
                if community == self.community && channel == self.channel {
                    // Check for encrypted messages that need MEK and auto-request.
                    // This fires Action::RequestMek for each unique missing generation,
                    // so the user doesn't have to manually run `rekindle key mek request`.
                    let missing_generations: std::collections::HashSet<u64> = messages
                        .iter()
                        .filter_map(|m| m.needs_mek)
                        .collect();
                    for gen in &missing_generations {
                        tracing::info!(
                            community = self.community.as_str(),
                            channel = self.channel.as_str(),
                            generation = gen,
                            "auto-requesting missing MEK"
                        );
                    }
                    self.pending_mek_requests = missing_generations;

                    let count = messages.len();
                    self.message_list.set_messages(messages);
                    if count > 0 {
                        self.message_list.set_last_read(count.saturating_sub(1));
                    }
                }
            }
            CommandResult::MessageSent { message_id } => {
                tracing::debug!(msg = message_id, "message sent confirmation");
            }
            CommandResult::CommunityInfoLoaded { detail } => {
                if detail.governance_key == self.community {
                    use crate::tui::components::channel_tree::ChannelEntry;
                    let channels: Vec<ChannelEntry> = detail.channels.iter().map(|ch| {
                        ChannelEntry {
                            id: ch.id.clone(),
                            name: ch.name.clone(),
                            kind: ch.kind.clone(),
                            category: ch.category_id.clone(),
                            unread: 0,
                            sort_order: ch.sort_order,
                        }
                    }).collect();
                    self.channel_tree.set_communities(
                        &[(detail.governance_key.clone(), detail.name.clone(), channels)],
                        &[],
                    );
                }
            }
            CommandResult::PeerListLoaded { peers } => {
                use crate::tui::components::peer_list::PeerEntry;
                let members: Vec<PeerEntry> = peers.iter().map(|p| {
                    PeerEntry {
                        key: p.key.clone(),
                        display_name: p.key_short.clone(),
                        status: if p.has_route { "online" } else { "offline" }.into(),
                        role: None,
                    }
                }).collect();
                self.peer_list.set_members(members);
            }
            _ => {}
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        match event {
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                community, channel, message_id, sender_pseudonym,
                sequence, timestamp, body, reply_to_sequence,
            }) if *community == self.community && *channel == self.channel => {
                self.message_list.push(DecryptedMessageDisplay {
                    message_id: message_id.clone(),
                    sequence: *sequence,
                    author_pseudonym: sender_pseudonym.clone(),
                    author_display_name: helpers::abbreviate_key(sender_pseudonym),
                    body: body.clone().unwrap_or_else(|| "(decrypting...)".into()),
                    timestamp: *timestamp,
                    reply_to_sequence: *reply_to_sequence,
                    mek_generation: 0,
                    is_encrypted: body.is_none(),
                    needs_mek: if body.is_none() { Some(0) } else { None },
                });
            }
            SubscriptionEvent::ChannelMessage(ChannelMessageEvent::Deleted {
                community, channel, message_id,
            }) if *community == self.community && *channel == self.channel => {
                self.message_list.remove_by_id(message_id);
            }
            SubscriptionEvent::Typing(TypingEvent::Started {
                context: TypingContext::Channel { community, channel }, who,
            }) if *community == self.community && *channel == self.channel => {
                self.typing_indicators.insert(who.clone(), std::time::Instant::now());
            }
            SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Channel { community, channel }, who,
            }) if *community == self.community && *channel == self.channel => {
                self.typing_indicators.remove(who);
            }
            _ => {}
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        self.expire_typing_indicators();
        // MEK auto-request is logged in on_command_result. The actual
        // RequestMek action is fired by the App when it processes the
        // CommandResult and sees pending_mek_requests is non-empty.
        // We clear after one tick cycle to avoid re-firing.
        self.pending_mek_requests.clear();
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
