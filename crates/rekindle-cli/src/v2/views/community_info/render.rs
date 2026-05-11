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
    let channel_height = (detail.channels.len() as u16 + 2).min(area.height / 3);
    #[allow(clippy::cast_possible_truncation)]
    let role_height = (detail.roles.len() as u16 + 2).min(area.height / 4);

    let [meta_area, channels_area, roles_area] = Layout::vertical([
        Constraint::Length(8), Constraint::Length(channel_height), Constraint::Min(role_height),
    ]).areas(area);

    render_metadata(frame, meta_area, detail);
    render_channels(frame, channels_area, detail, view.selected_channel);
    render_roles(frame, roles_area, detail);
}

fn render_metadata(frame: &mut Frame, area: Rect, detail: &rekindle_types::display::CommunityDetail) {
    let gov_short = helpers::abbreviate_key(&detail.governance_key);
    let owner_short = helpers::abbreviate_key(&detail.owner_pseudonym);
    let created = helpers::format_timestamp(detail.created_at);
    let our_key = helpers::abbreviate_key(&detail.our_pseudonym);
    let roles_str = if detail.our_roles.is_empty() {
        "none".to_string()
    } else {
        detail.our_roles.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    };

    let lines = vec![
        Line::from(vec![Span::styled("  Name:         ", Style::new().dim()), Span::styled(&detail.name, Style::new().bold())]),
        if detail.description.is_empty() { Line::from("") }
        else { Line::from(vec![Span::styled("  Description:  ", Style::new().dim()), Span::raw(&detail.description)]) },
        Line::from(vec![
            Span::styled("  Members:      ", Style::new().dim()), Span::raw(detail.member_count.to_string()),
            Span::styled("      Channels: ", Style::new().dim()), Span::raw(detail.channels.len().to_string()),
        ]),
        Line::from(vec![Span::styled("  Governance:   ", Style::new().dim()), Span::raw(gov_short)]),
        Line::from(vec![
            Span::styled("  Owner:        ", Style::new().dim()), Span::raw(owner_short),
            Span::styled("  Created: ", Style::new().dim()), Span::raw(created),
        ]),
        Line::from(vec![
            Span::styled("  Your key:     ", Style::new().dim()), Span::raw(our_key),
            Span::styled("  Roles: ", Style::new().dim()), Span::raw(roles_str),
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
        ListItem::new(Line::from(vec![
            Span::raw(format!("{prefix}#{:<20} ", ch.name)),
            Span::styled(&ch.kind, Style::new().dim()),
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
