//! Friend list rendering — presence grouping, pending requests.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use super::{FriendListView, presence_rank};
use crate::v2::helpers;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(view: &mut FriendListView, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    let title = format!(" Friends ({}) ", view.friends.len());
    let block = Block::bordered().title(title).border_style(theme.focused_border());

    if !view.loaded {
        frame.render_widget(Paragraph::new("  Loading friend list...").style(theme.style("dim")).block(block), area);
        return;
    }

    if view.friends.is_empty() && view.pending_requests.is_empty() {
        frame.render_widget(
            Paragraph::new("  No friends yet.\n  Add one: rekindle friend add --target <key>")
                .style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let items = build_items(view);
    frame.render_stateful_widget(
        List::new(items).block(block).highlight_style(Style::new().reversed()),
        area, &mut view.list_state,
    );
}

fn build_items(view: &FriendListView) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut current_status: Option<&str> = None;

    let mut sorted = view.friends.clone();
    sorted.sort_by(|a, b| presence_rank(&a.status).cmp(&presence_rank(&b.status)).then(a.display_name.cmp(&b.display_name)));

    for friend in &sorted {
        let status = friend.status.as_str();
        if current_status != Some(status) {
            current_status = Some(status);
            let count = sorted.iter().filter(|f| f.status == status).count();
            let label = capitalize_first(status);
            items.push(ListItem::new(Line::from(Span::styled(format!(" {label} ({count})"), Style::new().bold().dim()))));
        }

        let (glyph, text_label) = presence_indicator(status, view.use_unicode);
        let name = helpers::sanitize_for_display(&friend.display_name);
        let nickname = friend.nickname.as_ref().map(|n| format!(" ({n})")).unwrap_or_default();

        let last_seen = friend.last_seen_ms.map(|ms| {
            #[allow(clippy::cast_possible_truncation)]
            let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("system clock").as_millis() as u64;
            format!("  {}", helpers::format_duration_ago(std::time::Duration::from_millis(now_ms.saturating_sub(ms))))
        }).unwrap_or_default();

        let route = if friend.has_route { "" } else { " [no route]" };

        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("   {glyph} {text_label} ")),
            Span::styled(format!("{name}{nickname}"), Style::new().bold()),
            Span::styled(format!("{last_seen}{route}"), Style::new().dim()),
        ])));
    }

    if !view.pending_requests.is_empty() {
        items.push(ListItem::new(Line::from("")));
        items.push(ListItem::new(Line::from(
            Span::styled(format!(" Pending Requests ({})", view.pending_requests.len()), Style::new().bold().dim()),
        )));
        for req in &view.pending_requests {
            let name = helpers::sanitize_for_display(&req.display_name);
            let key_short = helpers::abbreviate_key(&req.public_key);
            let mut lines = vec![Line::from(vec![
                Span::raw("   ← "), Span::styled(name, Style::new().bold()),
                Span::styled(format!(" ({key_short})"), Style::new().dim()),
            ])];
            let msg = helpers::sanitize_for_display(&req.message);
            if !msg.is_empty() {
                lines.push(Line::from(Span::styled(format!("     \"{msg}\""), Style::new().dim().italic())));
            }
            items.push(ListItem::new(lines));
        }
    }

    items
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

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => format!("{}{}", first.to_uppercase().collect::<String>(), chars.as_str()),
    }
}
