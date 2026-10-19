//! Friend list view — friends grouped by presence, the requests waiting for
//! us, the people we blocked, and our open invites (plan E2.1). A friend's
//! session-reset request is the host's review, shown by the intent
//! overlay.
//!
//! Layout:
//! ```text
//! ┌─ Friends (23) ────────────────────────────────────────────┐
//! │ Online (8)                                                │
//! │   ● [ONLINE] alice                                        │
//! │ Offline (12)                                              │
//! │   ○ [OFFLINE] dave  last seen: 2h ago                     │
//! │ Pending Requests (2)        [a]ccept [r]eject [b]lock     │
//! │   ← frank (ab12…cd34)                                     │
//! │ Blocked (1)                 [u]nblock                     │
//! │   ✕ mallory                                               │
//! │ Open Invites (1)            [c]ancel                      │
//! │   ✉ 3f2a… expires in 47h                                  │
//! └──────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::dto::friends::{BlockedUser, Friend, OutgoingInvite, PendingRequest};

use super::View;
use crate::action::{Action, CommandResult};
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::presence_fmt::{
    capitalize_status as capitalize_first, presence_indicator, presence_rank,
};
use crate::theme::ThemeManager;

/// What a selection index names.
enum Selected<'a> {
    Friend(&'a Friend),
    Request(&'a PendingRequest),
    Blocked(&'a BlockedUser),
    Invite(&'a OutgoingInvite),
}

/// Friend list view state.
pub struct FriendListView {
    focus: FocusRing,
    /// All friends, sorted by presence rank then name.
    friends: Vec<Friend>,
    /// The requests waiting for our answer.
    requests: Vec<PendingRequest>,
    /// Everyone we blocked.
    blocked: Vec<BlockedUser>,
    /// Our open invites.
    invites: Vec<OutgoingInvite>,
    /// List selection, over the selectable rows in section order.
    list_state: ListState,
    /// Unicode glyph support.
    use_unicode: bool,
    /// Whether initial data has been loaded from the transport.
    loaded: bool,
}

fn status_of(friend: &Friend) -> &'static str {
    friend.status.as_wire_str()
}

fn section(title: &str, count: usize, keys: &str) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled(format!(" {title} ({count})"), Style::new().bold().dim()),
        Span::styled(format!("    {keys}"), Style::new().dim()),
    ]))
}

impl FriendListView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::FriendList]),
            friends: Vec::new(),
            requests: Vec::new(),
            blocked: Vec::new(),
            invites: Vec::new(),
            list_state: ListState::default(),
            use_unicode,
            loaded: false,
        }
    }

    fn total(&self) -> usize {
        self.friends.len() + self.requests.len() + self.blocked.len() + self.invites.len()
    }

    fn selected(&self) -> Option<Selected<'_>> {
        let mut idx = self.list_state.selected()?;
        if let Some(f) = self.friends.get(idx) {
            return Some(Selected::Friend(f));
        }
        idx -= self.friends.len();
        if let Some(r) = self.requests.get(idx) {
            return Some(Selected::Request(r));
        }
        idx -= self.requests.len();
        if let Some(b) = self.blocked.get(idx) {
            return Some(Selected::Blocked(b));
        }
        idx -= self.blocked.len();
        self.invites.get(idx).map(Selected::Invite)
    }

    fn friend_items(&self, items: &mut Vec<ListItem<'static>>) {
        let mut current_status: Option<&str> = None;
        for friend in &self.friends {
            let status = status_of(friend);
            if current_status != Some(status) {
                current_status = Some(status);
                let count = self
                    .friends
                    .iter()
                    .filter(|f| status_of(f) == status)
                    .count();
                items.push(ListItem::new(Line::from(Span::styled(
                    format!(" {} ({count})", capitalize_first(status)),
                    Style::new().bold().dim(),
                ))));
            }
            let (glyph, text_label) = presence_indicator(status, self.use_unicode);
            let name = helpers::sanitize_for_display(&friend.display_name);
            let nickname = friend
                .nickname
                .as_ref()
                .map(|n| format!(" ({})", helpers::sanitize_for_display(n)))
                .unwrap_or_default();
            let last_seen = friend
                .last_seen_at
                .and_then(|ms| u64::try_from(ms).ok())
                .map(|ms| {
                    let now_ms = rekindle_utils::timestamp_ms();
                    let elapsed = std::time::Duration::from_millis(now_ms.saturating_sub(ms));
                    format!("  {}", helpers::format_duration_ago(elapsed))
                })
                .unwrap_or_default();
            let pending = match (&friend.handshake_error, friend.friendship_state) {
                (Some(_), _) => " [secure session failed]",
                (None, rekindle_types::friend::FriendshipState::PendingOut) => " [request sent]",
                _ => "",
            };
            items.push(ListItem::new(Line::from(vec![
                Span::raw(format!("   {glyph} {text_label} ")),
                Span::styled(format!("{name}{nickname}"), Style::new().bold()),
                Span::styled(format!("{last_seen}{pending}"), Style::new().dim()),
            ])));
        }
    }

    fn other_items(&self, items: &mut Vec<ListItem<'static>>) {
        if !self.requests.is_empty() {
            items.push(section(
                "Pending Requests",
                self.requests.len(),
                "[a]ccept [r]eject [b]lock",
            ));
            for req in &self.requests {
                let mut lines = vec![Line::from(vec![
                    Span::raw("   ← "),
                    Span::styled(
                        helpers::sanitize_for_display(&req.display_name),
                        Style::new().bold(),
                    ),
                    Span::styled(
                        format!(" ({})", helpers::abbreviate_key(&req.public_key)),
                        Style::new().dim(),
                    ),
                ])];
                let msg = helpers::sanitize_for_display(&req.message);
                if !msg.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("     \"{msg}\""),
                        Style::new().dim().italic(),
                    )));
                }
                items.push(ListItem::new(lines));
            }
        }
        if !self.blocked.is_empty() {
            items.push(section("Blocked", self.blocked.len(), "[u]nblock"));
            for b in &self.blocked {
                let glyph = if self.use_unicode { "✕" } else { "x" };
                items.push(ListItem::new(Line::from(vec![
                    Span::raw(format!("   {glyph} ")),
                    Span::raw(helpers::sanitize_for_display(&b.display_name)),
                    Span::styled(
                        format!(" ({})", helpers::abbreviate_key(&b.public_key)),
                        Style::new().dim(),
                    ),
                ])));
            }
        }
        if !self.invites.is_empty() {
            items.push(section("Open Invites", self.invites.len(), "[c]ancel"));
            for inv in &self.invites {
                let glyph = if self.use_unicode { "✉" } else { "@" };
                let now_ms = rekindle_utils::timestamp_ms();
                let left = u64::try_from(inv.expires_at)
                    .unwrap_or(0)
                    .saturating_sub(now_ms)
                    / 3_600_000;
                items.push(ListItem::new(Line::from(vec![
                    Span::raw(format!("   {glyph} ")),
                    Span::raw(rekindle_utils::text::prefix(&inv.invite_id, 8).to_string()),
                    Span::styled(
                        format!("…  {}  expires in {left}h", inv.status),
                        Style::new().dim(),
                    ),
                ])));
            }
        }
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

        if self.total() == 0 {
            let para = Paragraph::new(concat!(
                "  No friends yet.\n  Add one: ",
                rekindle_client::cli_cmd!("friend add --target <key>")
            ))
            .style(Style::new().dim())
            .block(block);
            frame.render_widget(para, area);
            return Ok(());
        }

        let mut items = Vec::new();
        self.friend_items(&mut items);
        self.other_items(&mut items);
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        let max = self.total().saturating_sub(1);
        match action {
            Action::ScrollDown(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
            }
            Action::ScrollToTop => {
                if self.total() > 0 {
                    self.list_state.select(Some(0));
                }
            }
            Action::ScrollToBottom => {
                if self.total() > 0 {
                    self.list_state.select(Some(max));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::FriendListLoaded { friends } => {
                self.friends = friends;
                self.friends.sort_by(|a, b| {
                    presence_rank(status_of(a))
                        .cmp(&presence_rank(status_of(b)))
                        .then(a.display_name.cmp(&b.display_name))
                });
                self.loaded = true;
            }
            CommandResult::FriendExtrasLoaded {
                requests,
                blocked,
                invites,
            } => {
                self.requests = requests;
                self.blocked = blocked;
                self.invites = invites;
            }
            _ => return Ok(()),
        }
        let total = self.total();
        match self.list_state.selected() {
            None if total > 0 => self.list_state.select(Some(0)),
            Some(i) if i >= total => self.list_state.select(total.checked_sub(1)),
            _ => {}
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.total().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Enter => match self.selected()? {
                Selected::Friend(f) => Some(Action::ShowDmThread {
                    peer_key: f.public_key.clone(),
                }),
                _ => None,
            },
            KeyCode::Char('a') => match self.selected()? {
                Selected::Request(r) => Some(Action::AcceptFriendRequest {
                    public_key: r.public_key.clone(),
                    display_name: r.display_name.clone(),
                }),
                _ => None,
            },
            KeyCode::Char('r') => match self.selected()? {
                Selected::Request(r) => Some(Action::RejectFriendRequest(r.public_key.clone())),
                _ => None,
            },
            // Remove the selected friend; the host asks to confirm.
            KeyCode::Char('d') => match self.selected()? {
                Selected::Friend(f) => Some(Action::RemoveFriend {
                    peer_key: f.public_key.clone(),
                }),
                _ => None,
            },
            KeyCode::Char('b') => match self.selected()? {
                Selected::Friend(f) => Some(Action::BlockUser {
                    public_key: f.public_key.clone(),
                    display_name: f.display_name.clone(),
                }),
                Selected::Request(r) => Some(Action::BlockUser {
                    public_key: r.public_key.clone(),
                    display_name: r.display_name.clone(),
                }),
                _ => None,
            },
            KeyCode::Char('u') => match self.selected()? {
                Selected::Blocked(b) => Some(Action::UnblockUser {
                    public_key: b.public_key.clone(),
                }),
                _ => None,
            },
            KeyCode::Char('c') => match self.selected()? {
                Selected::Invite(i) => Some(Action::CancelInvite {
                    invite_id: i.invite_id.clone(),
                }),
                _ => None,
            },
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
