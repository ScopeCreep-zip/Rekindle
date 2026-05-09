//! Friend list view — friends grouped by presence with pending requests.
//!
//! Layout:
//! ```text
//! ┌─ Friends (23) ────────────────────────────────────────────┐
//! │ Online (8)                                                │
//! │   ● [ONLINE] alice                                        │
//! │   ● [ONLINE] bob                                          │
//! │ Away (3)                                                  │
//! │   ◐ [AWAY] carol    last seen: 5m ago                     │
//! │ Offline (12)                                              │
//! │   ○ [OFFLINE] dave  last seen: 2h ago                     │
//! ├─ Pending Requests (2) ────────────────────────────────────┤
//! │   ← frank (inbound)   [a]ccept [r]eject                  │
//! └──────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::display::FriendDisplay;

use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;
use super::View;

/// Friend list view state.
pub struct FriendListView {
    focus: FocusRing,
    /// All friends, sorted by presence rank then name.
    friends: Vec<FriendDisplay>,
    /// List selection state.
    list_state: ListState,
    /// Pending inbound friend requests.
    pending_requests: Vec<PendingRequestDisplay>,
    /// Unicode glyph support.
    use_unicode: bool,
    /// Whether initial data has been loaded from the transport.
    loaded: bool,
}

/// Pending friend request for display.
#[derive(Debug, Clone)]
pub struct PendingRequestDisplay {
    pub public_key: String,
    pub display_name: String,
    /// Message attached to the friend request — displayed to help the
    /// recipient make an informed accept/reject decision.
    pub message: String,
}

impl FriendListView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::FriendList]),
            friends: Vec::new(),
            list_state: ListState::default(),
            pending_requests: Vec::new(),
            use_unicode,
            loaded: false,
        }
    }

    fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let mut current_status: Option<&str> = None;

        for friend in &self.friends {
            let status = friend.status.as_str();
            if current_status != Some(status) {
                current_status = Some(status);
                let count = self.friends.iter().filter(|f| f.status == status).count();
                let label = capitalize_first(status);
                items.push(ListItem::new(Line::from(
                    Span::styled(
                        format!(" {label} ({count})"),
                        Style::new().bold().dim(),
                    ),
                )));
            }

            let (glyph, text_label) = presence_indicator(status, self.use_unicode);
            let name = helpers::sanitize_for_display(&friend.display_name);
            let nickname = friend
                .nickname
                .as_ref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();

            // Format last seen from epoch ms
            let last_seen = friend.last_seen_ms.map(|ms| {
                #[allow(clippy::cast_possible_truncation)]
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock")
                    .as_millis() as u64;
                let elapsed = std::time::Duration::from_millis(now_ms.saturating_sub(ms));
                format!("  {}", helpers::format_duration_ago(elapsed))
            }).unwrap_or_default();

            let route_indicator = if friend.has_route { "" } else { " [no route]" };

            let line = Line::from(vec![
                Span::raw(format!("   {glyph} {text_label} ")),
                Span::styled(format!("{name}{nickname}"), Style::new().bold()),
                Span::styled(format!("{last_seen}{route_indicator}"), Style::new().dim()),
            ]);
            items.push(ListItem::new(line));
        }

        if !self.pending_requests.is_empty() {
            items.push(ListItem::new(Line::from("")));
            items.push(ListItem::new(Line::from(
                Span::styled(
                    format!(" Pending Requests ({})", self.pending_requests.len()),
                    Style::new().bold().dim(),
                ),
            )));
            for req in &self.pending_requests {
                let name = helpers::sanitize_for_display(&req.display_name);
                let key_short = helpers::abbreviate_key(&req.public_key);
                let msg = helpers::sanitize_for_display(&req.message);
                let mut lines = vec![
                    Line::from(vec![
                        Span::raw("   ← "),
                        Span::styled(name, Style::new().bold()),
                        Span::styled(format!(" ({key_short})"), Style::new().dim()),
                    ]),
                ];
                if !msg.is_empty() {
                    lines.push(Line::from(
                        Span::styled(format!("     \"{msg}\""), Style::new().dim().italic()),
                    ));
                }
                items.push(ListItem::new(lines));
            }
        }

        items
    }
}

impl View for FriendListView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let title = format!(" Friends ({}) ", self.friends.len());
        let block = Block::bordered()
            .title(title)
            .border_style(theme.focused_border());

        if !self.loaded {
            let loading = Paragraph::new("  Loading friend list...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(loading, area);
            return Ok(());
        }

        if self.friends.is_empty() && self.pending_requests.is_empty() {
            let para = Paragraph::new(
                "  No friends yet.\n  Add one: rekindle friend add --target <key>",
            )
            .style(Style::new().dim())
            .block(block);
            frame.render_widget(para, area);
            return Ok(());
        }

        let items = self.build_items();
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        let total_visual = self.build_items().len();
        match action {
            Action::ScrollDown(_) => {
                let max = total_visual.saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
            }
            Action::ScrollToTop => {
                if total_visual > 0 {
                    self.list_state.select(Some(0));
                }
            }
            Action::ScrollToBottom => {
                if total_visual > 0 {
                    self.list_state.select(Some(total_visual - 1));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::FriendListLoaded { friends } = result {
            self.friends = friends;
            self.friends.sort_by(|a, b| {
                presence_rank(&a.status)
                    .cmp(&presence_rank(&b.status))
                    .then(a.display_name.cmp(&b.display_name))
            });
            if !self.friends.is_empty() && self.list_state.selected().is_none() {
                self.list_state.select(Some(0));
            }
            self.loaded = true;
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;

        // The visual list includes section headers interleaved with entries.
        // build_items() produces: [header, friend, friend, ..., header, friend, ...].
        // Map the visual selection index to the actual friend/request index.
        let total_visual_items = self.build_items().len();

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = total_visual_items.saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Enter => {
                let visual_idx = self.list_state.selected()?;
                // Count how many friend entries precede this visual index
                // by walking the same logic as build_items: headers don't count.
                let friend_idx = visual_to_friend_index(&self.friends, visual_idx)?;
                let friend = self.friends.get(friend_idx)?;
                Some(Action::ShowDmThread { peer_key: friend.public_key.clone() })
            }
            KeyCode::Char('a') => {
                let visual_idx = self.list_state.selected()?;
                let pending_idx = visual_to_pending_index(&self.friends, &self.pending_requests, visual_idx)?;
                let request = self.pending_requests.get(pending_idx)?;
                Some(Action::AcceptFriendRequest(request.public_key.clone()))
            }
            KeyCode::Char('r') => {
                let visual_idx = self.list_state.selected()?;
                let pending_idx = visual_to_pending_index(&self.friends, &self.pending_requests, visual_idx)?;
                let request = self.pending_requests.get(pending_idx)?;
                Some(Action::RejectFriendRequest(request.public_key.clone()))
            }
            _ => None,
        }
    }

    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        use rekindle_types::subscription_events::{SubscriptionEvent, FriendEvent, PresenceEvent};
        match event {
            SubscriptionEvent::Friend(FriendEvent::RequestReceived { from_key, display_name, message }) => {
                if !self.pending_requests.iter().any(|r| r.public_key == *from_key) {
                    self.pending_requests.push(PendingRequestDisplay {
                        public_key: from_key.clone(),
                        display_name: display_name.clone(),
                        message: message.clone(),
                    });
                }
            }
            SubscriptionEvent::Friend(FriendEvent::Accepted { peer_key, .. }) => {
                self.pending_requests.retain(|r| r.public_key != *peer_key);
            }
            SubscriptionEvent::Friend(FriendEvent::Removed { peer_key }) => {
                self.friends.retain(|f| f.public_key != *peer_key);
            }
            SubscriptionEvent::Presence(PresenceEvent::FriendChanged { peer_key, status, .. }) => {
                if let Some(f) = self.friends.iter_mut().find(|f| f.public_key == *peer_key) {
                    f.status.clone_from(status);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

fn presence_indicator(status: &str, unicode: bool) -> (&'static str, &'static str) {
    match status {
        "online" => (if unicode { "●" } else { "o" }, "[ONLINE]"),
        "away" => (if unicode { "◐" } else { "~" }, "[AWAY]"),
        "busy" => (if unicode { "●" } else { "-" }, "[BUSY]"),
        "offline" => (if unicode { "○" } else { "." }, "[OFFLINE]"),
        _ => (if unicode { "◌" } else { "?" }, "[?]"),
    }
}

fn presence_rank(status: &str) -> u8 {
    match status {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        "offline" => 3,
        _ => 4,
    }
}

/// Map a visual list index to a friend index, accounting for section headers.
/// Returns None if the visual index points at a header or is out of range.
fn visual_to_friend_index(friends: &[FriendDisplay], visual_idx: usize) -> Option<usize> {
    let mut current_status: Option<&str> = None;
    let mut visual = 0usize;

    for (friend_count, friend) in friends.iter().enumerate() {
        // Section header for new status group
        if current_status != Some(friend.status.as_str()) {
            current_status = Some(friend.status.as_str());
            if visual == visual_idx { return None; } // selected a header
            visual += 1;
        }
        if visual == visual_idx { return Some(friend_count); }
        visual += 1;
    }
    None
}

/// Map a visual list index to a pending request index, accounting for headers + friends.
/// Returns None if the visual index doesn't point at a pending request.
fn visual_to_pending_index(
    friends: &[FriendDisplay],
    pending: &[PendingRequestDisplay],
    visual_idx: usize,
) -> Option<usize> {
    // Count visual items for friends section
    let mut visual = 0usize;
    let mut current_status: Option<&str> = None;
    for friend in friends {
        if current_status != Some(friend.status.as_str()) {
            current_status = Some(friend.status.as_str());
            visual += 1; // header
        }
        visual += 1; // friend entry
    }

    if pending.is_empty() { return None; }

    // Empty line + "Pending Requests (N)" header = 2 visual items
    visual += 2;

    for (i, _req) in pending.iter().enumerate() {
        if visual == visual_idx { return Some(i); }
        visual += 1;
    }
    None
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
    }
}
