//! Tab bar component — top line navigation between views.
//!
//! Renders a horizontal tab bar showing: Dashboard, joined community
//! names, DMs, Friends. The active tab is highlighted with accent color
//! and bold. Tabs scroll with arrow indicators when they exceed the
//! terminal width.
//!
//! Source pattern:
//! - oxicord `presentation/ui/app.rs` — header with guild/channel context
//! - siggy `ui/status_bar.rs` — conversation indicator

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::theme::ThemeManager;

/// A tab entry for the bar.
#[derive(Debug, Clone)]
pub struct Tab {
    /// Display label.
    pub label: String,
    /// Unique identifier for this tab.
    pub id: String,
    /// Number of unread items (0 = no badge).
    pub unread: u32,
}

/// State for the tab bar.
pub struct TabBarState {
    /// All available tabs.
    pub tabs: Vec<Tab>,
    /// Index of the currently selected tab.
    pub selected: usize,
    /// Scroll offset when tabs exceed terminal width.
    scroll_offset: usize,
    /// Rendered tab x-ranges from the last draw: `(start_x, end_x, tab_index)`.
    /// Used for click-to-tab hit testing.
    pub click_regions: Vec<(u16, u16, usize)>,
}

impl TabBarState {
    /// Create a new tab bar with the given tabs.
    pub fn new(tabs: Vec<Tab>) -> Self {
        Self {
            tabs,
            selected: 0,
            scroll_offset: 0,
            click_regions: Vec::new(),
        }
    }

    /// Select the next tab. Wraps to first.
    pub fn next(&mut self) {
        if !self.tabs.is_empty() {
            self.selected = (self.selected + 1) % self.tabs.len();
            self.ensure_visible();
        }
    }

    /// Select the previous tab. Wraps to last.
    pub fn prev(&mut self) {
        if !self.tabs.is_empty() {
            self.selected = if self.selected == 0 {
                self.tabs.len() - 1
            } else {
                self.selected - 1
            };
            self.ensure_visible();
        }
    }

    /// Select a tab by index. No-op if out of bounds.
    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.selected = index;
            self.ensure_visible();
        }
    }

    /// ID of the currently selected tab.
    pub fn selected_id(&self) -> Option<&str> {
        self.tabs.get(self.selected).map(|t| t.id.as_str())
    }

    /// Select a tab by its ID string. No-op if not found.
    pub fn select_by_id(&mut self, id: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.selected = i;
            self.ensure_visible();
        }
    }

    /// Sync the tab bar selection to match the current view.
    ///
    /// This is the single source of truth coupling: the ViewKind determines
    /// which tab is highlighted. Called by App after every view transition.
    pub fn sync_to_view(&mut self, view: &crate::views::ViewKind) {
        use crate::views::ViewKind;
        let tab_id = match view {
            ViewKind::Dashboard | ViewKind::Doctor | ViewKind::IdentitySettings => "dashboard",
            ViewKind::DmInbox | ViewKind::DmThread { .. } => "dms",
            ViewKind::FriendList => "friends",
            ViewKind::ChannelWatch { .. }
            | ViewKind::CommunityInfo { .. }
            | ViewKind::VoiceSession { .. } => "communities",
        };
        self.select_by_id(tab_id);
    }

    /// Increment the unread count on a tab identified by its ID.
    /// No-op if the tab doesn't exist.
    pub fn increment_unread(&mut self, tab_id: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.unread = tab.unread.saturating_add(1);
        }
    }

    /// Reset the unread count on a tab. Called when the tab is selected.
    pub fn clear_unread(&mut self, tab_id: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.unread = 0;
        }
    }

    /// Hit-test a click position against rendered tab regions.
    /// Returns the tab index if the click is on a tab, None otherwise.
    pub fn click_tab(&self, column: u16, row: u16, tab_row: u16) -> Option<usize> {
        if row != tab_row {
            return None;
        }
        for &(start_x, end_x, idx) in &self.click_regions {
            if column >= start_x && column < end_x {
                return Some(idx);
            }
        }
        None
    }

    /// Ensure the selected tab is within the visible scroll window.
    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
        // We don't know the render width here, so we can't compute the
        // exact visible range. The render function handles overflow by
        // showing scroll indicators.
    }
}

/// Render the tab bar.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut TabBarState,
    theme: &ThemeManager,
) {
    state.click_regions.clear();
    if state.tabs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" rekindle", Style::new().bold())),
            area,
        );
        return;
    }

    let mut spans: Vec<Span<'_>> = Vec::new();
    let mut used_width: u16 = 0;
    let max_width = area.width;

    // Scroll indicator if we're not at the start
    if state.scroll_offset > 0 {
        spans.push(Span::styled("◀ ", Style::new().dim()));
        used_width += 2;
    }

    let mut visible_end = state.tabs.len();

    for (i, tab) in state.tabs.iter().enumerate().skip(state.scroll_offset) {
        let is_selected = i == state.selected;

        // Build label with optional unread badge
        let label = if tab.unread > 0 {
            format!(" {} ({}) ", tab.label, tab.unread)
        } else {
            format!(" {} ", tab.label)
        };

        #[allow(clippy::cast_possible_truncation)]
        let label_width = label.len() as u16;

        // Check if this tab fits
        if used_width + label_width + 2 > max_width {
            visible_end = i;
            break;
        }

        // Record click region: x start (relative to area.x) to x end
        let tab_start_x = area.x + used_width;
        state.click_regions.push((tab_start_x, tab_start_x + label_width, i));

        let style = if is_selected {
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("accent.primary"))
                .bold()
        } else {
            Style::new().dim()
        };

        spans.push(Span::styled(label, style));
        spans.push(Span::raw("│"));
        used_width += label_width + 1;
    }

    // Scroll indicator if there are more tabs beyond the visible range
    if visible_end < state.tabs.len() {
        spans.push(Span::styled(" ▶", Style::new().dim()));
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tabs() -> Vec<Tab> {
        vec![
            Tab { label: "Dashboard".into(), id: "dash".into(), unread: 0 },
            Tab { label: "dev-team".into(), id: "com1".into(), unread: 2 },
            Tab { label: "gaming".into(), id: "com2".into(), unread: 0 },
            Tab { label: "DMs".into(), id: "dms".into(), unread: 5 },
        ]
    }

    #[test]
    fn next_wraps() {
        let mut bar = TabBarState::new(test_tabs());
        assert_eq!(bar.selected, 0);
        bar.next();
        assert_eq!(bar.selected, 1);
        bar.next();
        bar.next();
        bar.next();
        assert_eq!(bar.selected, 0); // wrapped
    }

    #[test]
    fn prev_wraps() {
        let mut bar = TabBarState::new(test_tabs());
        bar.prev();
        assert_eq!(bar.selected, 3); // wrapped to last
    }

    #[test]
    fn select_by_index() {
        let mut bar = TabBarState::new(test_tabs());
        bar.select(2);
        assert_eq!(bar.selected, 2);
        assert_eq!(bar.selected_id(), Some("com2"));
    }

    #[test]
    fn select_out_of_bounds_is_noop() {
        let mut bar = TabBarState::new(test_tabs());
        bar.select(99);
        assert_eq!(bar.selected, 0);
    }

    #[test]
    fn selected_id_returns_correct() {
        let bar = TabBarState::new(test_tabs());
        assert_eq!(bar.selected_id(), Some("dash"));
    }

    #[test]
    fn empty_tabs() {
        let bar = TabBarState::new(Vec::new());
        assert_eq!(bar.selected_id(), None);
    }
}
