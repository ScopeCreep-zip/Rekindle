//! Peer list rendering — presence grouping, glyphs, role badges.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem};
use ratatui::Frame;

use super::state::{PeerList, capitalize_status};
use crate::v2::helpers::sanitize_for_display;

impl PeerList {
    pub fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let mut current_status: Option<&str> = None;

        for member in &self.members {
            let status = member.status.as_str();
            if current_status != Some(status) {
                current_status = Some(status);
                let count = self.members.iter().filter(|m| m.status == status).count();
                items.push(ListItem::new(Line::from(
                    Span::styled(format!(" {} ({count})", capitalize_status(status)), Style::new().bold().dim()),
                )));
            }

            let (glyph, _label) = presence_indicator(status, self.use_unicode);
            let name = sanitize_for_display(&member.display_name);
            let role_badge = member.role.as_ref().map(|r| format!(" [{r}]")).unwrap_or_default();

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {glyph} "), Style::new().dim()),
                Span::raw(name),
                Span::styled(role_badge, Style::new().dim()),
            ])));
        }
        items
    }

    pub fn draw_list(&mut self, frame: &mut Frame, area: Rect) {
        let online = self.members.iter().filter(|m| m.status == "online").count();
        let title = format!(" Members ({online}/{}) ", self.len());
        let block = Block::bordered()
            .title(title)
            .border_style(if self.is_focused { Style::new() } else { Style::new().dim() });

        if self.is_empty() {
            frame.render_widget(
                ratatui::widgets::Paragraph::new("  No members").style(Style::new().dim()).block(block),
                area,
            );
            return;
        }

        let items = self.build_items();
        frame.render_stateful_widget(List::new(items).block(block), area, &mut self.list_state);
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
