//! Peer list — right sidebar showing community members.
//!
//! Displays community members under the groups the Host decided
//! (`CommunityMemberView::group`: hoisted roles, Online, Offline), in the
//! Host's order, as the desktop does. Each member shows a presence glyph
//! + display name (never color alone).
//!
//! Source patterns:
//! - oxicord `presentation/widgets/guilds_tree.rs` — tree rendering
//! - siggy `ui/sidebar.rs` — presence grouping, status indicators

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use super::Component;
use crate::action::Action;
use crate::helpers;
use crate::presence_fmt::presence_indicator;
use rekindle_types::dto::community::{CommunityMemberView, MemberGroup};

/// A member entry for display.
#[derive(Debug, Clone)]
pub struct PeerEntry {
    /// Pseudonym key (abbreviated for display).
    pub key: String,
    /// Display name.
    pub display_name: String,
    /// Presence status string: "online", "away", "busy", "offline", "unknown".
    pub status: String,
    /// Role label (e.g., "owner", "admin").
    pub role: Option<String>,
    /// The member-list group the Host put this member in.
    pub group: MemberGroup,
}

impl From<CommunityMemberView> for PeerEntry {
    fn from(m: CommunityMemberView) -> Self {
        let role =
            (!m.display_role.is_empty() && m.display_role != "@everyone").then_some(m.display_role);
        Self {
            key: m.pseudonym_key,
            display_name: m.display_name,
            status: m.status,
            role,
            group: m.group,
        }
    }
}

/// A group's header text.
fn group_label(group: &MemberGroup) -> &str {
    match group {
        MemberGroup::Role { name, .. } => name,
        MemberGroup::Online => "Online",
        MemberGroup::Offline => "Offline",
    }
}

/// Peer list component.
pub struct PeerList {
    /// All members.
    members: Vec<PeerEntry>,
    /// Ratatui list state.
    list_state: ListState,
    /// Whether this component is focused.
    is_focused: bool,
    /// Whether Unicode glyphs are available.
    use_unicode: bool,
}

impl PeerList {
    /// Create a new empty peer list.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            members: Vec::new(),
            list_state: ListState::default(),
            is_focused: false,
            use_unicode,
        }
    }

    /// Replace the member list, kept in the Host's order.
    pub fn set_members(&mut self, members: Vec<PeerEntry>) {
        self.members = members;
    }

    /// Number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The list's lines: a header before each group, then its members.
    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut items = Vec::new();
        let mut current: Option<&MemberGroup> = None;

        for member in &self.members {
            if current != Some(&member.group) {
                current = Some(&member.group);
                let count = self
                    .members
                    .iter()
                    .filter(|m| m.group == member.group)
                    .count();
                let header = format!(" {} ({count})", group_label(&member.group));
                items.push(Line::from(Span::styled(header, Style::new().bold().dim())));
            }

            let (glyph, _label) = presence_indicator(&member.status, self.use_unicode);
            let name = helpers::sanitize_for_display(&member.display_name);
            let role_badge = member
                .role
                .as_ref()
                .map(|r| format!(" [{r}]"))
                .unwrap_or_default();

            let key_short = helpers::abbreviate_key(&member.key);
            let line = Line::from(vec![
                Span::styled(format!("  {glyph} "), Style::new().dim()),
                Span::raw(name),
                Span::styled(role_badge, Style::new().dim()),
                Span::styled(format!(" {key_short}"), Style::new().dim()),
            ]);
            items.push(line);
        }

        items
    }

    /// Build list items under the Host's groups.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        self.build_lines().into_iter().map(ListItem::new).collect()
    }
}

impl Component for PeerList {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.members.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> anyhow::Result<()> {
        let title = format!(" Members ({}) ", self.len());
        let block = Block::bordered()
            .title(title)
            .border_style(if self.is_focused {
                Style::new()
            } else {
                Style::new().dim()
            });

        if self.is_empty() {
            let empty = ratatui::widgets::Paragraph::new("  No members")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(empty, area);
            return Ok(());
        }

        let items = self.build_items();
        let list = List::new(items).block(block);
        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, name: &str, status: &str, group: MemberGroup) -> PeerEntry {
        PeerEntry {
            key: key.into(),
            display_name: name.into(),
            status: status.into(),
            role: None,
            group,
        }
    }

    /// The Host's order: the admin group, then Online, then Offline. Bob is
    /// offline yet the Host put him under Online, so the list must show
    /// him there: it renders the Host's groups, it does not regroup.
    fn test_members() -> Vec<PeerEntry> {
        let admins = MemberGroup::Role {
            id: 2,
            name: "Admins".into(),
        };
        vec![
            entry("pk1", "alice", "online", admins),
            entry("pk3", "carol", "online", MemberGroup::Online),
            entry("pk2", "bob", "offline", MemberGroup::Online),
            entry("pk4", "dave", "away", MemberGroup::Offline),
        ]
    }

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn renders_host_groups_in_host_order() {
        let mut list = PeerList::new(true);
        list.set_members(test_members());
        let lines: Vec<String> = list.build_lines().iter().map(text).collect();
        assert_eq!(lines[0], " Admins (1)");
        assert!(lines[1].contains("alice"));
        assert_eq!(lines[2], " Online (2)");
        assert!(lines[3].contains("carol"));
        assert!(lines[4].contains("bob"));
        assert_eq!(lines[5], " Offline (1)");
        assert!(lines[6].contains("dave"));
        assert_eq!(lines.len(), 7);
    }

    #[test]
    fn len_and_is_empty() {
        let mut list = PeerList::new(true);
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        list.set_members(test_members());
        assert!(!list.is_empty());
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn presence_indicator_unicode() {
        let (glyph, label) = presence_indicator("online", true);
        assert_eq!(glyph, "●");
        assert_eq!(label, "[ONLINE]");
    }

    #[test]
    fn presence_indicator_ascii() {
        let (glyph, label) = presence_indicator("online", false);
        assert_eq!(glyph, "o");
        assert_eq!(label, "[ONLINE]");
    }
}
