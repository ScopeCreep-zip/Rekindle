//! Community info rendering — metadata, channels with selection, roles.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::CommunityInfoView;
use crate::v2::helpers;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(view: &mut CommunityInfoView, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    if view.loading || view.detail.is_none() {
        let block = Block::bordered()
            .title(format!(" Community: {} ", helpers::abbreviate_key(&view.community)))
            .border_style(theme.focused_border());
        frame.render_widget(
            Paragraph::new("  Loading community details...").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let detail = view.detail.as_ref().expect("checked above");

    #[allow(clippy::cast_possible_truncation)]
    let channel_height = (detail.channels.len() as u16 + 2).min(area.height / 4);
    #[allow(clippy::cast_possible_truncation)]
    let role_height = (detail.roles.len() as u16 + 2).min(area.height / 5);
    #[allow(clippy::cast_possible_truncation)]
    let member_height = (detail.members.len() as u16 + 2).min(area.height / 4);
    #[allow(clippy::cast_possible_truncation)]
    let game_height = if view.game_servers.is_empty() { 0 } else { (view.game_servers.len() as u16 + 2).min(area.height / 5) };

    let [meta_area, channels_area, members_area, game_area, roles_area] = Layout::vertical([
        Constraint::Length(8),
        Constraint::Length(channel_height),
        Constraint::Length(member_height),
        Constraint::Length(game_height),
        Constraint::Min(role_height),
    ]).areas(area);

    render_metadata(frame, meta_area, detail);
    render_channels(frame, channels_area, detail, view.selected_channel);
    render_members(frame, members_area, detail);
    if !view.game_servers.is_empty() {
        render_game_servers(frame, game_area, &view.game_servers);
    }
    render_roles(frame, roles_area, detail);
}

fn render_metadata(frame: &mut Frame, area: Rect, detail: &rekindle_types::display::CommunityDetail) {
    let gov_short = helpers::abbreviate_key(&detail.governance_key);
    let owner_short = helpers::abbreviate_key(&detail.owner_pseudonym);
    let created = if detail.created_at == 0 { "unknown".to_string() } else { helpers::format_timestamp(detail.created_at) };
    let our_key = helpers::abbreviate_key(&detail.our_pseudonym);
    let operator_badge = if detail.is_operator { " [operator]" } else { "" };

    let lines = vec![
        Line::from(vec![Span::styled("  Name:         ", Style::new().dim()), Span::styled(&detail.name, Style::new().bold())]),
        if detail.description.is_empty() { Line::from("") }
        else { Line::from(vec![Span::styled("  Description:  ", Style::new().dim()), Span::raw(&detail.description)]) },
        Line::from(vec![
            Span::styled("  Members:      ", Style::new().dim()), Span::raw(detail.member_count.to_string()),
            Span::styled("  Channels: ", Style::new().dim()), Span::raw(detail.channels.len().to_string()),
            Span::styled("  Roles: ", Style::new().dim()), Span::raw(detail.roles.len().to_string()),
        ]),
        Line::from(vec![Span::styled("  Governance:   ", Style::new().dim()), Span::raw(gov_short)]),
        Line::from(vec![
            Span::styled("  Owner:        ", Style::new().dim()), Span::raw(owner_short),
            Span::styled("  Created: ", Style::new().dim()), Span::raw(created),
        ]),
        Line::from(vec![
            Span::styled("  Your key:     ", Style::new().dim()), Span::raw(our_key),
            Span::styled(operator_badge, Style::new().bold()),
            Span::styled("  Policy: ", Style::new().dim()), Span::raw(&detail.join_policy),
        ]),
    ];

    let block = Block::bordered().title(format!(" {} ", detail.name));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_channels(frame: &mut Frame, area: Rect, detail: &rekindle_types::display::CommunityDetail, selected: usize) {
    let title = format!(" Channels ({}) — j/k navigate, Enter to open ", detail.channels.len());
    let block = Block::bordered().title(title).border_style(Style::new().dim());

    if detail.channels.is_empty() {
        frame.render_widget(Paragraph::new("  No channels.").style(Style::new().dim()).block(block), area);
        return;
    }

    let items: Vec<ListItem<'_>> = detail.channels.iter().enumerate().map(|(i, ch)| {
        let topic = if ch.topic.is_empty() { String::new() } else { format!("  — {}", ch.topic) };
        let prefix = if i == selected { "▸ " } else { "  " };
        let kind_str = format!("{:?}", ch.kind);
        ListItem::new(Line::from(vec![
            Span::raw(format!("{prefix}#{:<20} ", ch.name)),
            Span::styled(kind_str, Style::new().dim()),
            Span::styled(topic, Style::new().dim()),
        ]))
    }).collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));
    frame.render_stateful_widget(
        List::new(items).block(block).highlight_style(Style::new().reversed()),
        area, &mut list_state,
    );
}

fn render_members(frame: &mut Frame, area: Rect, detail: &rekindle_types::display::CommunityDetail) {
    let title = format!(" Members ({}) ", detail.members.len());
    let block = Block::bordered().title(title).border_style(Style::new().dim());

    if detail.members.is_empty() {
        frame.render_widget(Paragraph::new("  No members.").style(Style::new().dim()).block(block), area);
        return;
    }

    let lines: Vec<Line<'_>> = detail.members.iter().map(|m| {
        let (status_glyph, status_label) = match m.status.as_str() {
            "online" => ("●", "[ONLINE]"),
            "away" => ("◐", "[AWAY]"),
            "busy" => ("●", "[BUSY]"),
            "offline" => ("○", "[OFFLINE]"),
            _ => ("○", "[OFFLINE]"),
        };
        let role = m.role_name.as_deref().map_or(String::new(), |r| format!("  [{r}]"));
        let timeout = m.member.timeout_until.map_or(String::new(), |t| format!("  [timeout until {t}]"));
        Line::from(vec![
            Span::raw(format!("  {status_glyph} {status_label} ")),
            Span::styled(&m.member.display_name, Style::new().bold()),
            Span::styled(role, Style::new().dim()),
            Span::styled(timeout, Style::new().dim()),
        ])
    }).collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_game_servers(frame: &mut Frame, area: Rect, servers: &[serde_json::Value]) {
    let title = format!(" 🎮 Game Servers ({}) ", servers.len());
    let block = Block::bordered().title(title).border_style(Style::new().dim());

    let lines: Vec<Line<'_>> = servers.iter().map(|s| {
        let label = s.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let address = s.get("address").and_then(|v| v.as_str()).unwrap_or("?");
        let game = s.get("game_id").and_then(|v| v.as_str()).unwrap_or("?");
        let by = s.get("added_by").and_then(|v| v.as_str()).unwrap_or("?");
        let by_short = helpers::abbreviate_key(by);
        Line::from(vec![
            Span::raw(format!("  🎮 {label} — ")),
            Span::styled(address, Style::new().bold()),
            Span::styled(format!("  ({game})  added by {by_short}"), Style::new().dim()),
        ])
    }).collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_roles(frame: &mut Frame, area: Rect, detail: &rekindle_types::display::CommunityDetail) {
    let title = format!(" Roles ({}) ", detail.roles.len());
    let block = Block::bordered().title(title).border_style(Style::new().dim());

    if detail.roles.is_empty() {
        frame.render_widget(Paragraph::new("  No roles defined.").style(Style::new().dim()).block(block), area);
        return;
    }

    let lines: Vec<Line<'_>> = detail.roles.iter().map(|r| {
        Line::from(vec![
            Span::raw(format!("  {} ", r.name)),
            Span::styled(format!("(id: {}, pos: {}, perms: 0x{:X})", r.id, r.position, r.permissions), Style::new().dim()),
        ])
    }).collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
