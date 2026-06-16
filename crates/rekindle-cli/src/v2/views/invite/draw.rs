//! Invite list rendering — active invites with usage, expiry, and scroll.

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
    let targets = caches.click_targets.entry(ViewKindTag::Invite).or_default();
    targets.insert(FocusId::MessageList, area);

    let name = state.communities.name_for(community);
    let block = Block::bordered()
        .title(format!(" Invites: {name} \u{2014} [n]ew  [r]evoke "))
        .border_style(theme.focused_border());

    let invites = state.communities.invites.get(community);
    if invites.is_none() || invites.map_or(false, |v| v.is_empty()) {
        frame.render_widget(
            Paragraph::new("  No active invites. Press [n] to create one.")
                .style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let invites = invites.unwrap();
    let items: Vec<ListItem<'static>> = invites.iter().map(|inv| {
        let code = inv.get("code").or_else(|| inv.get("invite_code"))
            .and_then(|v| v.as_str()).unwrap_or("?");
        let max = inv.get("maxUses").or_else(|| inv.get("max_uses"))
            .and_then(|v| v.as_u64());
        let uses = inv.get("useCount").or_else(|| inv.get("use_count"))
            .and_then(|v| v.as_u64()).unwrap_or(0);
        let uses_str = match max {
            Some(0) | None => format!("{uses}/unlimited"),
            Some(m) => format!("{uses}/{m}"),
        };
        let expires = inv.get("expiresAt").or_else(|| inv.get("expires_at"))
            .and_then(|v| v.as_u64());
        let expires_str = match expires {
            Some(0) | None => "never".to_string(),
            Some(ts) => helpers::format_timestamp(ts, &state.timezone),
        };
        let code_short = if code.len() > 12 { format!("{}...", &code[..12]) } else { code.to_string() };
        let uses_label = format!("uses: {uses_str:<12} ");
        let uses_span = theme.span("dim", &uses_label);
        let exp_label = format!("expires: {expires_str}");
        let exp_span = theme.span("dim", &exp_label);
        ListItem::new(Line::from(vec![
            Span::raw(format!("  {code_short:<16} ")),
            uses_span,
            exp_span,
        ]))
    }).collect();

    let item_count = items.len();
    let max = item_count.saturating_sub(1);
    let selected = state.ephemeral.invite_selected;
    caches.invite_list.select(Some(selected.min(max)));

    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(theme.style("selected"))
            .highlight_symbol("\u{25b8} ")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
            .scroll_padding(1),
        area,
        &mut caches.invite_list,
    );
    let mut scrollbar_state = ScrollbarState::new(item_count)
        .position(caches.invite_list.selected().unwrap_or(0));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area,
        &mut scrollbar_state,
    );
}
