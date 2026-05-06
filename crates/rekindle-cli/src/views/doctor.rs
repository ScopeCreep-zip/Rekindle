//! Doctor view — interactive diagnostic check results.
//!
//! Renders `Vec<Check>` from the doctor module with category grouping,
//! text labels `[PASS]`/`[WARN]`/`[FAIL]` (never color alone), and
//! remediation hints for non-passing checks. Supports `r` to rerun
//! and `/` to filter by category.
//!
//! Layout:
//! ```text
//! ┌─ Doctor ───────────────────────────────────────────────────┐
//! │ NODE                                                       │
//! │   [PASS] node.running                attached               │
//! │   [PASS] node.public_internet        ready                  │
//! │ CRYPTO                                                     │
//! │   [WARN] crypto.prekeys.low          3 remaining            │
//! │     replenish: rekindle key prekeys replenish                │
//! │ NETWORK                                                    │
//! │   [FAIL] network.peers.reachable     2/8 reachable          │
//! │     check: rekindle network routes --refresh                 │
//! ├────────────────────────────────────────────────────────────┤
//! │  4 pass, 1 warn, 1 fail   [r] rerun  [/] filter  [q]      │
//! └────────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use rekindle_types::display::{Check, CheckStatus};
use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;
use super::View;

/// Doctor view state.
pub struct DoctorView {
    focus: FocusRing,
    /// All check results.
    checks: Vec<Check>,
    /// List selection state.
    list_state: ListState,
    /// Active category filter (None = show all).
    category_filter: Option<String>,
    /// Whether checks are currently running.
    running: bool,
    /// Unicode glyph support.
    use_unicode: bool,
}

impl DoctorView {
    /// Create a new doctor view.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DoctorList]),
            checks: Vec::new(),
            list_state: ListState::default(),
            category_filter: None,
            running: false,
            use_unicode,
        }
    }

    /// Filtered checks based on the active category filter.
    fn filtered_checks(&self) -> Vec<&Check> {
        match &self.category_filter {
            None => self.checks.iter().collect(),
            Some(cat) => self
                .checks
                .iter()
                .filter(|c| c.category == cat.as_str())
                .collect(),
        }
    }

    /// Build list items for rendering.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        let filtered = self.filtered_checks();
        let mut items = Vec::new();
        let mut current_category: Option<&str> = None;

        for check in &filtered {
            if current_category != Some(check.category.as_str()) {
                current_category = Some(check.category.as_str());
                items.push(ListItem::new(Line::from(
                    Span::styled(
                        format!(" {}", check.category.to_uppercase()),
                        Style::new().bold(),
                    ),
                )));
            }

            let (icon, icon_style) = match check.status {
                CheckStatus::Pass => (
                    if self.use_unicode { "✓ [PASS]" } else { "[PASS]" },
                    Style::new().bold(),
                ),
                CheckStatus::Warn => (
                    if self.use_unicode { "⚠ [WARN]" } else { "[WARN]" },
                    Style::new().bold(),
                ),
                CheckStatus::Fail => (
                    if self.use_unicode { "✗ [FAIL]" } else { "[FAIL]" },
                    Style::new().bold(),
                ),
            };

            let line = Line::from(vec![
                Span::raw("   "),
                Span::styled(icon, icon_style),
                Span::raw(format!(" {:<35} ", check.id)),
                Span::raw(check.value.clone()),
            ]);
            items.push(ListItem::new(line));

            // Remediation hint for non-passing checks
            if check.status != CheckStatus::Pass && !check.description.is_empty() {
                for hint_line in check.description.lines() {
                    items.push(ListItem::new(Line::from(
                        Span::styled(
                            format!("     {hint_line}"),
                            Style::new().dim(),
                        ),
                    )));
                }
            }
        }

        items
    }

    /// Summary counts: (pass, warn, fail).
    fn summary(&self) -> (usize, usize, usize) {
        let filtered = self.filtered_checks();
        let pass = filtered.iter().filter(|c| c.status == CheckStatus::Pass).count();
        let warn = filtered.iter().filter(|c| c.status == CheckStatus::Warn).count();
        let fail = filtered.iter().filter(|c| c.status == CheckStatus::Fail).count();
        (pass, warn, fail)
    }
}

impl View for DoctorView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [list_area, summary_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // Check list
        let filter_label = self
            .category_filter
            .as_ref()
            .map(|c| format!(" [filter: {c}]"))
            .unwrap_or_default();

        let title = if self.running {
            format!(" Doctor (running...){filter_label} ")
        } else {
            format!(" Doctor ({}){filter_label} ", self.checks.len())
        };

        let block = Block::bordered()
            .title(title)
            .border_style(theme.focused_border());

        if self.checks.is_empty() {
            let para = Paragraph::new("  Loading diagnostics...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, list_area);
        } else {
            let items = self.build_items();
            let list = List::new(items)
                .block(block)
                .highlight_style(Style::new().reversed());
            frame.render_stateful_widget(list, list_area, &mut self.list_state);
        }

        // Summary line
        let (pass, warn, fail) = self.summary();
        let summary = Line::from(vec![
            Span::raw(format!("  {pass} passed, {warn} warnings, {fail} failures")),
            Span::styled("   [r] rerun  [/] filter  [q] back", Style::new().dim()),
        ]);
        frame.render_widget(Paragraph::new(summary), summary_area);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::Refresh => {
                self.running = true;
                // The App will spawn the doctor check task and deliver
                // CommandResult::StatusLoaded when done.
                return Ok(Some(Action::ShowDoctor));
            }
            Action::ScrollDown(_) => {
                let max = self.build_items().len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
            }
            Action::ScrollToTop => {
                self.list_state.select(Some(0));
            }
            Action::ScrollToBottom => {
                let max = self.build_items().len().saturating_sub(1);
                self.list_state.select(Some(max));
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::StatusLoaded { snapshot } = result {
            self.checks = snapshot.checks;
            self.running = false;
            if !self.checks.is_empty() && self.list_state.selected().is_none() {
                self.list_state.select(Some(0));
            }
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
