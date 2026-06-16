//! Community info rendering — name, description, channels, members, roles.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    community: &str,
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::CommunityInfo).or_default();
    targets.insert(FocusId::CommunityInfoPanel, area);

    let name = state.communities.name_for(community);
    let border = theme.focused_border();
    let block = Block::bordered().title(format!(" {name} ")).border_style(border);

    let detail = state.communities.details.get(community);
    if detail.is_none() {
        frame.render_widget(
            Paragraph::new("  Loading community info...").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }
    let detail = detail.unwrap();

    let [info_area, members_area] = Layout::horizontal([
        Constraint::Percentage(60),
        Constraint::Percentage(40),
    ]).areas(block.inner(area));
    frame.render_widget(block, area);

    let mut info_lines = vec![
        Line::from(vec![
            theme.span("dim", "  Name: "),
            Span::styled(&detail.name, theme.style("accent").add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            theme.span("dim", "  Description: "),
            Span::raw(&detail.description),
        ]),
        Line::from(vec![
            theme.span("dim", "  Owner: "),
            Span::raw(&detail.owner_pseudonym),
        ]),
        Line::from(vec![
            theme.span("dim", "  Join Policy: "),
            Span::raw(&detail.join_policy),
        ]),
        Line::from(vec![
            theme.span("dim", "  Channels: "),
            Span::raw(detail.channel_count.to_string()),
        ]),
        Line::from(vec![
            theme.span("dim", "  Roles: "),
            Span::raw(detail.role_count.to_string()),
        ]),
        Line::from(vec![
            theme.span("dim", "  Members: "),
            Span::raw(detail.member_count.to_string()),
        ]),
        Line::from(""),
    ];

    for ch in &detail.channels {
        let unread = detail.channel_unreads.get(&ch.id).copied().unwrap_or(0);
        let badge = if unread > 0 { format!(" ({unread})") } else { String::new() };
        let badge_span = theme.span("accent", &badge);
        info_lines.push(Line::from(vec![
            Span::raw(format!("  #{}", ch.name)),
            badge_span,
        ]));
    }

    frame.render_widget(Paragraph::new(info_lines), info_area);

    if let Some(members) = state.communities.members.get(community) {
        let member_lines: Vec<Line<'_>> = members.iter().map(|m| {
            let role = m.role_name.as_ref().map(|r| format!(" [{r}]")).unwrap_or_default();
            let status = format!(" ({})", m.status);
            let role_span = theme.span("dim", &role);
            let status_span = theme.span("dim", &status);
            Line::from(vec![
                Span::raw(format!("  {}", m.member.display_name)),
                role_span,
                status_span,
            ])
        }).collect();
        let block = Block::bordered()
            .title(format!(" Members ({}) ", members.len()))
            .border_style(theme.unfocused_border());
        frame.render_widget(Paragraph::new(member_lines).block(block), members_area);
    }
}
