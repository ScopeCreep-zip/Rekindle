//! Search overlay state — query, filtered results, selection.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::ListState;

use super::filter::{SearchItem, filter_items};
use super::super::super::action::{Action, SearchMode};

/// Search overlay state.
pub struct SearchOverlay {
    pub mode: SearchMode,
    pub query: String,
    pub items: Vec<SearchItem>,
    pub filtered_indices: Vec<usize>,
    pub list_state: ListState,
    pub visible: bool,
}

impl SearchOverlay {
    pub fn new() -> Self {
        Self {
            mode: SearchMode::QuickSwitch, query: String::new(),
            items: Vec::new(), filtered_indices: Vec::new(),
            list_state: ListState::default(), visible: false,
        }
    }

    pub fn open(&mut self, mode: SearchMode, items: Vec<SearchItem>) {
        self.mode = mode;
        self.query.clear();
        self.items = items;
        self.visible = true;
        self.refresh_filter();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.items.clear();
        self.filtered_indices.clear();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => { self.close(); Some(Action::CloseOverlay) }
            KeyCode::Enter => {
                let idx = self.selected_item_index()?;
                let action = self.items[idx].action.clone();
                self.close();
                Some(action)
            }
            KeyCode::Down | KeyCode::Tab => { self.move_selection(1); None }
            KeyCode::Up | KeyCode::BackTab => { self.move_selection(-1); None }
            KeyCode::Char(c) => { self.query.push(c); self.refresh_filter(); None }
            KeyCode::Backspace => { self.query.pop(); self.refresh_filter(); None }
            _ => None,
        }
    }

    fn refresh_filter(&mut self) {
        self.filtered_indices = filter_items(&self.items, &self.query);
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered_indices.is_empty() { return; }
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

    fn selected_item_index(&self) -> Option<usize> {
        self.list_state.selected().and_then(|i| self.filtered_indices.get(i)).copied()
    }
}
