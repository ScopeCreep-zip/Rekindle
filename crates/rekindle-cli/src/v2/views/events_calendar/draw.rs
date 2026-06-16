//! Event calendar rendering — list of community events with scroll.

use ratatui::layout::Rect;
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
    community: &str,
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::Events).or_default();
    targets.insert(FocusId::MessageList, area);

    let name = state.communities.name_for(community);
    let block = Block::bordered()
        .title(format!(" Events: {name} "))
        .border_style(theme.focused_border());

    let events = state.communities.events.get(community);
    if events.is_none() || events.map_or(false, |v| v.is_empty()) {
        frame.render_widget(
            Paragraph::new("  No events.").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let events = events.unwrap();
    let items: Vec<ListItem<'static>> = events.iter().map(|ev| {
        let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let start = ev.get("startTime").or_else(|| ev.get("start_time"))
            .and_then(|v| v.as_u64())
            .map(|ts| helpers::format_timestamp(ts, &state.timezone))
            .unwrap_or_default();
        let start_label = format!("  {start}");
        let start_span = theme.span("dim", &start_label);
        ListItem::new(Line::from(vec![
            Span::raw(format!("  {title}")),
            start_span,
        ]))
    }).collect();

    let item_count = items.len();
    let max = item_count.saturating_sub(1);
    let selected = state.ephemeral.events_selected;
    caches.events_list.select(Some(selected.min(max)));

    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(theme.style("selected"))
            .highlight_symbol("\u{25b8} ")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
            .scroll_padding(1),
        area,
        &mut caches.events_list,
    );
    let mut scrollbar_state = ScrollbarState::new(item_count)
        .position(caches.events_list.selected().unwrap_or(0));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area,
        &mut scrollbar_state,
    );
}
