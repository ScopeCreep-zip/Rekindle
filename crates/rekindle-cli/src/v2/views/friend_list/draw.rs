//! Friend list rendering — presence-grouped list with pending requests.

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::v2::helpers;
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
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::FriendList).or_default();
    targets.insert(FocusId::FriendList, area);

    let title = format!(" Friends ({}) ", state.friends.friends.len());
    let border = theme.focused_border();
    let block = Block::bordered().title(title).border_style(border);

    if !state.friends.loaded {
        frame.render_widget(
            Paragraph::new("  Loading friend list...").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    if state.friends.friends.is_empty() && state.friends.pending_requests.is_empty() {
        frame.render_widget(
            Paragraph::new("  No friends yet.\n  Add one: rekindle friend add --target <key>")
                .style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let mut items: Vec<ListItem<'static>> = Vec::new();
    let mut current_status: Option<&str> = None;

    for friend in &state.friends.friends {
        let status = friend.status.as_str();
        if current_status != Some(status) {
            current_status = Some(status);
            let count = state.friends.friends.iter().filter(|f| f.status == status).count();
            let label = capitalize_first(status);
            items.push(ListItem::new(Line::from(
                Span::styled(format!(" {label} ({count})"), theme.style("dim").add_modifier(Modifier::BOLD)),
            )));
        }
        let (glyph, _, _) = theme.presence_indicator(status);
        let name = helpers::sanitize_for_display(&friend.display_name);
        let nickname = friend.nickname.as_ref().map(|n| format!(" ({n})")).unwrap_or_default();
        let route = if friend.has_route { "" } else { " [no route]" };

        let label = format!("{name}{nickname}");
        let label_span = theme.span("accent", &label);
        let route_span = theme.span("dim", route);
        items.push(ListItem::new(Line::from(vec![
            Span::raw(format!("   {glyph} ")),
            label_span,
            route_span,
        ])));
    }

    if !state.friends.pending_requests.is_empty() {
        items.push(ListItem::new(Line::from("")));
        items.push(ListItem::new(Line::from(
            Span::styled(
                format!(" Pending Requests ({})", state.friends.pending_requests.len()),
                theme.style("dim").add_modifier(Modifier::BOLD),
            ),
        )));
        for req in &state.friends.pending_requests {
            let name = helpers::sanitize_for_display(&req.display_name);
            let key_short = helpers::abbreviate_key(&req.public_key);
            let age = if req.received_at > 0 && state.wall_clock_ms > req.received_at {
                let elapsed = std::time::Duration::from_millis(state.wall_clock_ms - req.received_at);
                format!("  {}", helpers::format_duration_ago(elapsed))
            } else {
                String::new()
            };
            let name_span = theme.span("accent", &name);
            let detail = format!(" ({key_short})");
            let detail_span = theme.span("dim", &detail);
            let age_span = theme.span("dim", &age);
            items.push(ListItem::new(Line::from(vec![
                Span::raw("   \u{2190} "),
                name_span,
                detail_span,
                age_span,
            ])));
        }
    }

    let item_count = items.len();
    let selected_index = state.session.friend_selected_key.as_deref()
        .and_then(|pk| state.friends.friends.iter().position(|f| f.public_key == pk));
    caches.friend_list.select(selected_index);

    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(theme.style("selected"))
            .highlight_symbol("\u{25b8} ")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
            .scroll_padding(1),
        area,
        &mut caches.friend_list,
    );
    let mut scrollbar_state = ScrollbarState::new(item_count)
        .position(caches.friend_list.selected().unwrap_or(0));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area,
        &mut scrollbar_state,
    );
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => format!("{}{}", first.to_uppercase().collect::<String>(), chars.as_str()),
    }
}
