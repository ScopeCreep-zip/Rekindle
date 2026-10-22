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

/// One line of the list: a section header (never selectable) or an entry,
/// by its index in the section's vector. Draw, selection and the cursor all
/// read the same rows, so a key acts on the highlighted entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Header {
        title: String,
        count: usize,
        keys: &'static str,
    },
    Friend(usize),
    Request(usize),
    Blocked(usize),
    Invite(usize),
}

impl Row {
    const fn is_entry(&self) -> bool {
        !matches!(self, Self::Header { .. })
    }
}

/// A non-friend section: its title, entry count, key hints and row maker.
type Section = (&'static str, usize, &'static str, fn(usize) -> Row);

/// What the selected row names.
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
    /// The list's rows, rebuilt whenever the data changes.
    rows: Vec<Row>,
    /// List selection, an index into `rows` that always names an entry.
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
    let mut spans = vec![Span::styled(
        format!(" {title} ({count})"),
        Style::new().bold().dim(),
    )];
    if !keys.is_empty() {
        spans.push(Span::styled(format!("    {keys}"), Style::new().dim()));
    }
    ListItem::new(Line::from(spans))
}

impl FriendListView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::FriendList]),
            friends: Vec::new(),
            requests: Vec::new(),
            blocked: Vec::new(),
            invites: Vec::new(),
            rows: Vec::new(),
            list_state: ListState::default(),
            use_unicode,
            loaded: false,
        }
    }

    fn total(&self) -> usize {
        self.friends.len() + self.requests.len() + self.blocked.len() + self.invites.len()
    }

    /// The rows in display order: friends under one header per presence,
    /// then each non-empty section under its header.
    fn build_rows(&self) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.total() + 8);
        let mut current_status: Option<&str> = None;
        for (i, friend) in self.friends.iter().enumerate() {
            let status = status_of(friend);
            if current_status != Some(status) {
                current_status = Some(status);
                let count = self
                    .friends
                    .iter()
                    .filter(|f| status_of(f) == status)
                    .count();
                rows.push(Row::Header {
                    title: capitalize_first(status),
                    count,
                    keys: "",
                });
            }
            rows.push(Row::Friend(i));
        }
        let sections: [Section; 3] = [
            (
                "Pending Requests",
                self.requests.len(),
                "[a]ccept [r]eject [b]lock",
                Row::Request,
            ),
            ("Blocked", self.blocked.len(), "[u]nblock", Row::Blocked),
            ("Open Invites", self.invites.len(), "[c]ancel", Row::Invite),
        ];
        for (title, count, keys, row) in sections {
            if count > 0 {
                rows.push(Row::Header {
                    title: title.to_string(),
                    count,
                    keys,
                });
                rows.extend((0..count).map(row));
            }
        }
        rows
    }

    /// Rebuild the rows and keep the selection on an entry.
    fn refresh_rows(&mut self) {
        self.rows = self.build_rows();
        let at = self
            .list_state
            .selected()
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        let entry = self
            .entry_from(at, true)
            .or_else(|| self.entry_from(at, false));
        self.list_state.select(entry);
    }

    /// The first entry at `from` or beyond, forward or backward.
    fn entry_from(&self, from: usize, forward: bool) -> Option<usize> {
        if forward {
            (from..self.rows.len()).find(|&i| self.rows[i].is_entry())
        } else {
            (0..=from.min(self.rows.len().checked_sub(1)?))
                .rev()
                .find(|&i| self.rows[i].is_entry())
        }
    }

    /// Move the cursor to the next entry down or up; it stays put at the
    /// last entry either way.
    fn step(&mut self, down: bool) {
        let Some(i) = self.list_state.selected() else {
            self.list_state.select(self.entry_from(0, true));
            return;
        };
        let next = if down {
            self.entry_from(i + 1, true)
        } else {
            i.checked_sub(1).and_then(|j| self.entry_from(j, false))
        };
        if let Some(n) = next {
            self.list_state.select(Some(n));
        }
    }

    fn selected(&self) -> Option<Selected<'_>> {
        match self.rows.get(self.list_state.selected()?)? {
            Row::Header { .. } => None,
            Row::Friend(i) => self.friends.get(*i).map(Selected::Friend),
            Row::Request(i) => self.requests.get(*i).map(Selected::Request),
            Row::Blocked(i) => self.blocked.get(*i).map(Selected::Blocked),
            Row::Invite(i) => self.invites.get(*i).map(Selected::Invite),
        }
    }

    fn row_item(&self, row: &Row) -> ListItem<'static> {
        match row {
            Row::Header { title, count, keys } => section(title, *count, keys),
            Row::Friend(i) => self.friend_item(&self.friends[*i]),
            Row::Request(i) => request_item(&self.requests[*i]),
            Row::Blocked(i) => self.blocked_item(&self.blocked[*i]),
            Row::Invite(i) => self.invite_item(&self.invites[*i]),
        }
    }

    fn friend_item(&self, friend: &Friend) -> ListItem<'static> {
        let (glyph, text_label) = presence_indicator(status_of(friend), self.use_unicode);
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
        let pending = if friend.identity_pin_mismatch {
            // A trust alert (plan E2.4, B3): nothing is sent to them.
            " [identity key mismatch]"
        } else if friend.handshake_error.is_some() {
            " [secure session failed]"
        } else if friend.handshake_retrying {
            " [retrying with a fresh key]"
        } else if friend.friendship_state == rekindle_types::friend::FriendshipState::PendingOut {
            " [request sent]"
        } else {
            ""
        };
        ListItem::new(Line::from(vec![
            Span::raw(format!("   {glyph} {text_label} ")),
            Span::styled(format!("{name}{nickname}"), Style::new().bold()),
            Span::styled(format!("{last_seen}{pending}"), Style::new().dim()),
        ]))
    }

    fn blocked_item(&self, b: &BlockedUser) -> ListItem<'static> {
        let glyph = if self.use_unicode { "✕" } else { "x" };
        ListItem::new(Line::from(vec![
            Span::raw(format!("   {glyph} ")),
            Span::raw(helpers::sanitize_for_display(&b.display_name)),
            Span::styled(
                format!(" ({})", helpers::abbreviate_key(&b.public_key)),
                Style::new().dim(),
            ),
        ]))
    }

    fn invite_item(&self, inv: &OutgoingInvite) -> ListItem<'static> {
        let glyph = if self.use_unicode { "✉" } else { "@" };
        let now_ms = rekindle_utils::timestamp_ms();
        let left = u64::try_from(inv.expires_at)
            .unwrap_or(0)
            .saturating_sub(now_ms)
            / 3_600_000;
        ListItem::new(Line::from(vec![
            Span::raw(format!("   {glyph} ")),
            Span::raw(rekindle_utils::text::prefix(&inv.invite_id, 8).to_string()),
            Span::styled(
                format!("…  {}  expires in {left}h", inv.status),
                Style::new().dim(),
            ),
        ]))
    }
}

fn request_item(req: &PendingRequest) -> ListItem<'static> {
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
    ListItem::new(lines)
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

        let items: Vec<ListItem<'static>> =
            self.rows.iter().map(|row| self.row_item(row)).collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) => self.step(true),
            Action::ScrollUp(_) => self.step(false),
            Action::ScrollToTop => self.list_state.select(self.entry_from(0, true)),
            Action::ScrollToBottom => {
                let last = self.rows.len().saturating_sub(1);
                self.list_state.select(self.entry_from(last, false));
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
        self.refresh_rows();
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.step(true);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.step(false);
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
                }),
                Selected::Request(r) => Some(Action::BlockUser {
                    public_key: r.public_key.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rekindle_types::friend::FriendshipState;
    use rekindle_types::friend::UserStatus;

    fn friend(key: &str, status: UserStatus) -> Friend {
        Friend {
            public_key: key.to_string(),
            display_name: key.to_string(),
            nickname: None,
            status,
            status_message: None,
            game_info: None,
            group: None,
            group_label: rekindle_types::dto::friends::DEFAULT_FRIEND_GROUP.to_string(),
            last_seen_at: None,
            friendship_state: FriendshipState::Accepted,
            handshake_error: None,
            handshake_retrying: false,
            identity_pin_mismatch: false,
        }
    }

    fn loaded() -> FriendListView {
        let mut view = FriendListView::new(true);
        view.on_command_result(CommandResult::FriendListLoaded {
            friends: vec![
                friend("bob", UserStatus::Offline),
                friend("alice", UserStatus::Online),
                friend("carol", UserStatus::Online),
            ],
        })
        .unwrap();
        view.on_command_result(CommandResult::FriendExtrasLoaded {
            requests: vec![PendingRequest {
                public_key: "frank".into(),
                display_name: "frank".into(),
                message: String::new(),
                received_at: 0,
            }],
            blocked: vec![BlockedUser {
                public_key: "mallory".into(),
                display_name: "mallory".into(),
                blocked_at: 0,
            }],
            invites: vec![OutgoingInvite {
                invite_id: "inv1".into(),
                url: String::new(),
                created_at: 0,
                expires_at: 0,
                status: "pending".into(),
                accepted_by: None,
            }],
        })
        .unwrap();
        view
    }

    fn press(view: &mut FriendListView, c: char) -> Option<Action> {
        view.handle_focused_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    #[test]
    fn keys_act_on_the_highlighted_entry() {
        let mut view = loaded();
        // Online(2) alice carol, Offline(1) bob, three one-entry sections.
        assert_eq!(view.rows.len(), 11);
        assert_eq!(view.list_state.selected(), Some(1));
        assert!(
            matches!(press(&mut view, 'd'), Some(Action::RemoveFriend { peer_key }) if peer_key == "alice")
        );
        press(&mut view, 'j');
        press(&mut view, 'j');
        // The Offline header is skipped.
        assert_eq!(view.list_state.selected(), Some(4));
        assert!(
            matches!(press(&mut view, 'd'), Some(Action::RemoveFriend { peer_key }) if peer_key == "bob")
        );
        press(&mut view, 'j');
        assert!(matches!(
            press(&mut view, 'a'),
            Some(Action::AcceptFriendRequest { public_key, .. }) if public_key == "frank"
        ));
        press(&mut view, 'j');
        assert!(matches!(
            press(&mut view, 'u'),
            Some(Action::UnblockUser { public_key }) if public_key == "mallory"
        ));
        press(&mut view, 'j');
        assert!(matches!(
            press(&mut view, 'c'),
            Some(Action::CancelInvite { invite_id }) if invite_id == "inv1"
        ));
        // Past the last entry the cursor stays; up skips headers back.
        press(&mut view, 'j');
        assert_eq!(view.list_state.selected(), Some(10));
        press(&mut view, 'k');
        assert_eq!(view.list_state.selected(), Some(8));
        view.update(Action::ScrollToTop).unwrap();
        assert_eq!(view.list_state.selected(), Some(1));
        press(&mut view, 'k');
        assert_eq!(view.list_state.selected(), Some(1));
        view.update(Action::ScrollToBottom).unwrap();
        assert_eq!(view.list_state.selected(), Some(10));
    }

    #[test]
    fn a_reload_keeps_the_cursor_off_headers() {
        let mut view = loaded();
        view.update(Action::ScrollToBottom).unwrap();
        view.on_command_result(CommandResult::FriendExtrasLoaded {
            requests: Vec::new(),
            blocked: Vec::new(),
            invites: Vec::new(),
        })
        .unwrap();
        assert_eq!(view.rows.len(), 5);
        assert_eq!(view.list_state.selected(), Some(4));
        assert!(view.selected().is_some());
    }
}
