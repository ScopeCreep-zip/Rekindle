//! Search overlay state — substring filter with match indices for highlight.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchMode {
    QuickSwitch,
    MessageSearch,
    CommandPalette,
}

#[derive(Clone, Debug)]
pub struct SearchItem {
    pub label: String,
    pub detail: String,
    /// The input handler maps this to an `Effect`.
    pub action_tag: SearchActionTag,
}

#[derive(Clone, Debug)]
pub enum SearchActionTag {
    /// Navigate to a view.
    Navigate(super::navigation::ViewKind),
    /// Scroll the current message list to a specific message.
    ScrollToMessage { message_id: String },
    /// Open a file in the file preview.
    FileSelected { path: String },
    /// Open a search overlay in the specified mode.
    OpenSearch(SearchMode),
    /// Switch the active theme at runtime.
    SetTheme { name: String },
    /// Toggle a boolean setting.
    Toggle(ToggleAction),
    /// Copy text to clipboard.
    CopyToClipboard(String),
    /// Pin a theme to the current community.
    PinCommunityTheme { community: String, theme: String },
    /// Pin a theme to a DM thread.
    PinDmTheme { peer_key: String, theme: String },
    /// Remove a community theme pin (revert to user default).
    UnpinCommunityTheme { community: String },
    /// Remove a DM theme pin.
    UnpinDmTheme { peer_key: String },
}

/// Toggleable settings exposed through the command palette.
#[derive(Clone, Debug)]
pub enum ToggleAction {
    Sidebar,
    Timezone,
    Help,
}

/// Substring search with match indices for per-character highlight rendering.
#[derive(Debug)]
pub struct SearchState {
    pub mode: SearchMode,
    pub query: String,
    pub items: Vec<SearchItem>,
    /// Indices into `items` that match the current query.
    pub filtered_indices: Vec<usize>,
    /// Parallel to `filtered_indices`. Each entry holds all byte-start
    /// positions where the query appears in the item's label. Multiple
    /// occurrences per label are all recorded for highlight rendering.
    pub match_positions: Vec<Vec<usize>>,
    pub visible: bool,
}

impl SearchState {
    pub fn new(mode: SearchMode) -> Self {
        Self {
            mode,
            query: String::new(),
            items: Vec::new(),
            filtered_indices: Vec::new(),
            match_positions: Vec::new(),
            visible: true,
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
        self.match_positions.clear();
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refresh_filter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refresh_filter();
    }

    pub fn move_selection(&self, delta: i32, list_state: &mut ratatui::widgets::ListState) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let max = self.filtered_indices.len() - 1;
        let current = list_state.selected().unwrap_or(0);
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let new = if delta > 0 {
            (current + delta as usize).min(max)
        } else {
            current.saturating_sub((-delta) as usize)
        };
        list_state.select(Some(new));
    }

    /// Returns the item index in `items` (not the filtered index).
    pub fn selected_item_index(&self, list_state: &ratatui::widgets::ListState) -> Option<usize> {
        list_state.selected()
            .and_then(|i| self.filtered_indices.get(i))
            .copied()
    }

    /// Query byte length, for highlight span computation in the renderer.
    pub fn query_byte_len(&self) -> usize {
        self.query.len()
    }

    /// Substring filter with all-occurrence match position tracking.
    pub fn refresh_filter(&mut self) {
        self.filtered_indices.clear();
        self.match_positions.clear();

        if self.query.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
            self.match_positions = vec![Vec::new(); self.items.len()];
        } else {
            let q = self.query.to_lowercase();
            for (i, item) in self.items.iter().enumerate() {
                let label_lower = item.label.to_lowercase();
                let positions = find_all_occurrences(&label_lower, &q);
                if !positions.is_empty() {
                    self.filtered_indices.push(i);
                    self.match_positions.push(positions);
                } else if item.detail.to_lowercase().contains(&q) {
                    self.filtered_indices.push(i);
                    self.match_positions.push(Vec::new());
                }
            }
        }
    }
}

/// Find all byte-start positions of `needle` in `haystack`.
fn find_all_occurrences(haystack: &str, needle: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        positions.push(start + pos);
        start += pos + 1;
        if start >= haystack.len() {
            break;
        }
    }
    positions
}
