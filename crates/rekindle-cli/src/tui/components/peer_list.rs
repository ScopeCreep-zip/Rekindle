//! Peer list — right sidebar showing community members.
//!
//! Displays community members grouped by presence status:
//! Online, Away, Busy, Offline. Each member shows a presence glyph
//! + text label (never color alone) + display name.
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
use crate::helpers;
use crate::tui::action::Action;

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

    /// Replace the member list.
    pub fn set_members(&mut self, members: Vec<PeerEntry>) {
        self.members = members;
        // Sort by presence: online first, then away, busy, offline, unknown
        self.members.sort_by(|a, b| {
            presence_rank(&a.status)
                .cmp(&presence_rank(&b.status))
                .then(a.display_name.cmp(&b.display_name))
        });
    }

    /// Number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Build list items grouped by presence status.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let mut current_status: Option<&str> = None;

        for member in &self.members {
            let status = member.status.as_str();
            if current_status != Some(status) {
                current_status = Some(status);
                let count = self.members.iter().filter(|m| m.status == status).count();
                let header = format!(" {} ({count})", capitalize_status(status));
                items.push(ListItem::new(Line::from(Span::styled(
                    header,
                    Style::new().bold().dim(),
                ))));
            }

            let (glyph, _label) = presence_indicator(status, self.use_unicode);
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
            items.push(ListItem::new(line));
        }

        items
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

/// Presence indicator: returns (glyph, text_label).
/// Always provides both — color is applied by the caller via theme tokens.
fn presence_indicator(status: &str, unicode: bool) -> (&'static str, &'static str) {
    match status {
        "online" => {
            if unicode {
                ("●", "[ONLINE]")
            } else {
                ("o", "[ONLINE]")
            }
        }
        "away" => {
            if unicode {
                ("◐", "[AWAY]")
            } else {
                ("~", "[AWAY]")
            }
        }
        "busy" => {
            if unicode {
                ("●", "[BUSY]")
            } else {
                ("-", "[BUSY]")
            }
        }
        "offline" => {
            if unicode {
                ("○", "[OFFLINE]")
            } else {
                (".", "[OFFLINE]")
            }
        }
        _ => {
            if unicode {
                ("◌", "[?]")
            } else {
                ("?", "[?]")
            }
        }
    }
}

/// Sort rank for presence status — lower = higher priority.
fn presence_rank(status: &str) -> u8 {
    match status {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        "offline" => 3,
        _ => 4,
    }
}

/// Capitalize the first letter of a status string.
fn capitalize_status(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_members() -> Vec<PeerEntry> {
        vec![
            PeerEntry {
                key: "pk1".into(),
                display_name: "alice".into(),
                status: "online".into(),
                role: Some("admin".into()),
            },
            PeerEntry {
                key: "pk2".into(),
                display_name: "bob".into(),
                status: "offline".into(),
                role: None,
            },
            PeerEntry {
                key: "pk3".into(),
                display_name: "carol".into(),
                status: "online".into(),
                role: None,
            },
        ]
    }

    #[test]
    fn set_members_sorts_by_presence() {
        let mut list = PeerList::new(true);
        list.set_members(test_members());
        // Online members should come first
        assert_eq!(list.members[0].status, "online");
        assert_eq!(list.members[1].status, "online");
        assert_eq!(list.members[2].status, "offline");
    }

    #[test]
    fn len_and_is_empty() {
        let mut list = PeerList::new(true);
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        list.set_members(test_members());
        assert!(!list.is_empty());
        assert_eq!(list.len(), 3);
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

    #[test]
    fn presence_rank_ordering() {
        assert!(presence_rank("online") < presence_rank("away"));
        assert!(presence_rank("away") < presence_rank("busy"));
        assert!(presence_rank("busy") < presence_rank("offline"));
        assert!(presence_rank("offline") < presence_rank("unknown"));
    }

    #[test]
    fn capitalize_works() {
        assert_eq!(capitalize_status("online"), "Online");
        assert_eq!(capitalize_status(""), "");
    }
}
