//! Doctor rendering — diagnostic checks table with category grouping and health meters.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use rekindle_types::display::{Check, CheckStatus};

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
    let targets = caches.click_targets.entry(ViewKindTag::Doctor).or_default();
    targets.insert(FocusId::DoctorList, area);

    let checks = match state.status_snapshot.as_ref() {
        Some(snap) => &snap.checks,
        None => {
            let block = Block::bordered().title(" Doctor ").border_style(theme.focused_border());
            frame.render_widget(
                Paragraph::new("  Loading diagnostics...").style(theme.style("dim")).block(block),
                area,
            );
            return;
        }
    };

    let [list_area, summary_area, diag_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ]).areas(area);

    let title = format!(" Doctor ({}) ", checks.len());
    let block = Block::bordered().title(title).border_style(theme.focused_border());

    let items = build_items(checks, theme);

    if caches.doctor_list.selected().is_none() && !items.is_empty() {
        caches.doctor_list.select(Some(0));
    }

    let item_count = items.len();
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(theme.style("selected"))
            .highlight_symbol("\u{25b8} ")
            .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
            .scroll_padding(1),
        list_area,
        &mut caches.doctor_list,
    );
    let mut scrollbar_state = ScrollbarState::new(item_count)
        .position(caches.doctor_list.selected().unwrap_or(0));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        list_area,
        &mut scrollbar_state,
    );

    let pass = checks.iter().filter(|c| c.status == CheckStatus::Pass).count();
    let warn = checks.iter().filter(|c| c.status == CheckStatus::Warn).count();
    let fail = checks.iter().filter(|c| c.status == CheckStatus::Fail).count();

    frame.render_widget(Paragraph::new(Line::from(vec![
        Span::raw(format!("  {pass} passed, {warn} warnings, {fail} failures")),
        theme.span("dim", "   [r] rerun  [q] back"),
    ])), summary_area);

    let project_path = state.search_engine.as_ref()
        .and_then(|e| e.base_path())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "unknown".into());
    let diag_text = format!("  Theme: {} ({}) | Tier: {:?} (256: {}) | Unicode: {} | Project: {}",
        theme.name(),
        if theme.is_light() { "light" } else { "dark" },
        theme.tier(),
        theme.tier().has_256(),
        theme.use_unicode(),
        project_path,
    );
    let diag_span = theme.span("dim", &diag_text);
    frame.render_widget(Paragraph::new(Line::from(diag_span)), diag_area);
}

fn build_items<'a>(checks: &[Check], theme: &'a ThemeManager) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let mut current_category: Option<&str> = None;

    for check in checks {
        if current_category != Some(check.category.as_str()) {
            current_category = Some(check.category.as_str());
            items.push(ListItem::new(Line::from(Span::styled(
                format!(" {}", check.category.to_uppercase()),
                theme.style("accent").add_modifier(Modifier::BOLD),
            ))));
        }

        let use_unicode = theme.use_unicode();
        let (icon, icon_style) = match check.status {
            CheckStatus::Pass => (
                if use_unicode { "\u{2713} [PASS]" } else { "[PASS]" },
                theme.style("accent"),
            ),
            CheckStatus::Warn => (
                if use_unicode { "\u{26a0} [WARN]" } else { "[WARN]" },
                theme.style("dim").add_modifier(Modifier::BOLD),
            ),
            CheckStatus::Fail => (
                if use_unicode { "\u{2717} [FAIL]" } else { "[FAIL]" },
                theme.style("error"),
            ),
        };

        items.push(ListItem::new(Line::from(vec![
            Span::raw("   "),
            Span::styled(icon, icon_style),
            Span::raw(format!(" {:<35} ", check.id)),
            Span::raw(check.value.clone()),
        ])));

        if check.status != CheckStatus::Pass && !check.description.is_empty() {
            for hint_line in check.description.lines() {
                let hint = format!("     {hint_line}");
                let hint_span = theme.span("dim", &hint);
                items.push(ListItem::new(Line::from(hint_span)));
            }
        }
    }
    items
}
