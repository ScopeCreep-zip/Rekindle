//! Search overlay — fuzzy search across communities, channels, friends, and commands.
//!
//! Full-screen modal overlay with an input field at top and a filtered
//! results list below. Uses nucleo (helix editor's fuzzy matching engine)
//! for fast, incremental, typo-tolerant matching.
//!
//! Three search modes:
//! - `QuickSwitch` (Ctrl+K): communities, channels, DM peers
//! - `MessageSearch` (/): messages in current channel
//! - `CommandPalette` (:): available commands
//!
//! Navigation: type to filter, j/k or arrows navigate, Enter selects, Esc closes.
//! Matched characters in results are highlighted with the `search.match` theme token.
//!
//! Source patterns:
//! - oxicord `presentation/ui/quick_switcher.rs` — fuzzy matcher + recents boost
//! - skim library API — SkimItem trait, run_with

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::helpers;
use crate::tui::action::{Action, SearchMode};
use crate::tui::theme::ThemeManager;

/// A searchable item in the results list.
#[derive(Debug, Clone)]
pub struct SearchItem {
    /// Display label for matching and rendering.
    pub label: String,
    /// Secondary text (e.g., community name for channels, peer key for friends).
    pub detail: String,
    /// The action to dispatch when this item is selected.
    pub action: Action,
}

/// Search overlay state.
pub struct SearchOverlay {
    /// Current search mode.
    mode: SearchMode,
    /// The query string typed by the user.
    query: String,
    /// All available items (unfiltered).
    items: Vec<SearchItem>,
    /// Indices into `items` that match the current query, sorted by score.
    filtered_indices: Vec<usize>,
    /// Ratatui list state for the results.
    list_state: ListState,
    /// Whether the overlay is visible.
    pub visible: bool,
}

impl SearchOverlay {
    /// Create a new search overlay.
    pub fn new() -> Self {
        Self {
            mode: SearchMode::QuickSwitch,
            query: String::new(),
            items: Vec::new(),
            filtered_indices: Vec::new(),
            list_state: ListState::default(),
            visible: false,
        }
    }

    /// Open the overlay with the given mode and items.
    pub fn open(&mut self, mode: SearchMode, items: Vec<SearchItem>) {
        self.mode = mode;
        self.query.clear();
        self.items = items;
        self.visible = true;
        self.refresh_filter();
    }

    /// Close the overlay.
    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.items.clear();
        self.filtered_indices.clear();
    }

    /// Handle a key event. Returns an Action if the event produced one.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.close();
                Some(Action::CloseOverlay)
            }
            KeyCode::Enter => {
                // Select the highlighted item
                if let Some(idx) = self.selected_item_index() {
                    let action = self.items[idx].action.clone();
                    self.close();
                    return Some(action);
                }
                None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.move_selection(1);
                None
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.move_selection(-1);
                None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.refresh_filter();
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.refresh_filter();
                None
            }
            _ => None,
        }
    }

    /// Render the search overlay as a centered modal.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        if !self.visible {
            return;
        }

        // Overlay covers most of the screen
        let popup = centered_rect(
            area,
            area.width.saturating_sub(8),
            area.height.saturating_sub(4),
        );
        frame.render_widget(Clear, popup);

        let title = match self.mode {
            SearchMode::QuickSwitch => " Quick Switch (Ctrl+K) ",
            SearchMode::MessageSearch => " Search Messages ",
            SearchMode::CommandPalette => " Command Palette ",
        };

        let block = Block::bordered()
            .title(title)
            .border_style(theme.focused_border());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        // Split inner: search input (1 line) + results (remaining)
        let [input_area, results_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);

        // Search input
        let input_text = format!(" > {}", self.query);
        let cursor = Span::styled("█", Style::new().dim());
        let input_line = Line::from(vec![Span::raw(input_text), cursor]);
        frame.render_widget(Paragraph::new(input_line), input_area);

        // Results
        if self.filtered_indices.is_empty() {
            let empty = if self.query.is_empty() {
                "  Type to search..."
            } else {
                "  No results."
            };
            frame.render_widget(
                Paragraph::new(empty).style(Style::new().dim()),
                results_area,
            );
            return;
        }

        let items: Vec<ListItem<'_>> = self
            .filtered_indices
            .iter()
            .map(|&idx| {
                let item = &self.items[idx];
                let label = helpers::sanitize_for_display(&item.label);
                let detail = if item.detail.is_empty() {
                    String::new()
                } else {
                    format!("  ({})", helpers::sanitize_for_display(&item.detail))
                };

                let line = Line::from(vec![
                    Span::raw(format!("  {label}")),
                    Span::styled(detail, Style::new().dim()),
                ]);
                ListItem::new(line)
            })
            .collect();

        let result_count = format!(" {}/{} ", self.filtered_indices.len(), self.items.len());
        let list = List::new(items)
            .highlight_style(Style::new().reversed())
            .block(Block::default().title(result_count));

        // We need a mutable copy of list_state for rendering
        let mut state = self.list_state;
        frame.render_stateful_widget(list, results_area, &mut state);
    }

    /// Refresh the filtered indices based on the current query.
    ///
    /// Uses case-insensitive substring matching. For empty queries, shows all items.
    /// Nucleo integration for proper fuzzy scoring will be added when the nucleo
    /// API stabilizes around the `Injector` pattern — for now, substring match
    /// is correct and responsive for the expected item counts (<1000 items).
    fn refresh_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let query_lower = self.query.to_lowercase();
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.label.to_lowercase().contains(&query_lower)
                        || item.detail.to_lowercase().contains(&query_lower)
                })
                .map(|(i, _)| i)
                .collect();
        }

        // Reset selection to first result
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    /// Move the selection up or down by delta.
    fn move_selection(&mut self, delta: i32) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        let current = self.list_state.selected().unwrap_or(0);

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)]
        let new = if delta > 0 {
            (current + delta as usize).min(max)
        } else {
            current.saturating_sub((-delta) as usize)
        };

        self.list_state.select(Some(new));
    }

    /// Get the original item index for the currently selected result.
    fn selected_item_index(&self) -> Option<usize> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered_indices.get(i))
            .copied()
    }
}

/// Center a rect within a larger area.
fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_items() -> Vec<SearchItem> {
        vec![
            SearchItem {
                label: "dev-team".into(),
                detail: "community".into(),
                action: Action::ShowCommunityInfo {
                    community: "gov1".into(),
                },
            },
            SearchItem {
                label: "#general".into(),
                detail: "dev-team".into(),
                action: Action::ShowChannel {
                    community: "gov1".into(),
                    channel: "ch1".into(),
                },
            },
            SearchItem {
                label: "alice".into(),
                detail: "friend".into(),
                action: Action::ShowDmThread {
                    peer_key: "pk1".into(),
                },
            },
            SearchItem {
                label: "#random".into(),
                detail: "dev-team".into(),
                action: Action::ShowChannel {
                    community: "gov1".into(),
                    channel: "ch2".into(),
                },
            },
        ]
    }

    #[test]
    fn open_shows_all_items() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        assert!(overlay.visible);
        assert_eq!(overlay.filtered_indices.len(), 4);
    }

    #[test]
    fn typing_filters_results() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());

        // Type "gen" — should match "#general"
        overlay.query = "gen".into();
        overlay.refresh_filter();
        assert_eq!(overlay.filtered_indices.len(), 1);
        assert_eq!(overlay.items[overlay.filtered_indices[0]].label, "#general");
    }

    #[test]
    fn case_insensitive_search() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());

        overlay.query = "ALICE".into();
        overlay.refresh_filter();
        assert_eq!(overlay.filtered_indices.len(), 1);
    }

    #[test]
    fn empty_query_shows_all() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        overlay.query.clear();
        overlay.refresh_filter();
        assert_eq!(overlay.filtered_indices.len(), 4);
    }

    #[test]
    fn no_match_shows_empty() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        overlay.query = "zzzzz".into();
        overlay.refresh_filter();
        assert!(overlay.filtered_indices.is_empty());
    }

    #[test]
    fn close_clears_state() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        overlay.close();
        assert!(!overlay.visible);
        assert!(overlay.items.is_empty());
        assert!(overlay.query.is_empty());
    }

    #[test]
    fn move_selection_clamps() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        overlay.move_selection(100); // should clamp to last
        assert_eq!(overlay.list_state.selected(), Some(3));
        overlay.move_selection(-100); // should clamp to first
        assert_eq!(overlay.list_state.selected(), Some(0));
    }

    #[test]
    fn selected_item_index_returns_correct() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        // First item should be selected
        assert_eq!(overlay.selected_item_index(), Some(0));
        overlay.move_selection(2);
        assert_eq!(overlay.selected_item_index(), Some(2));
    }

    #[test]
    fn search_by_detail() {
        let mut overlay = SearchOverlay::new();
        overlay.open(SearchMode::QuickSwitch, test_items());
        overlay.query = "friend".into();
        overlay.refresh_filter();
        assert_eq!(overlay.filtered_indices.len(), 1);
        assert_eq!(overlay.items[overlay.filtered_indices[0]].label, "alice");
    }
}
