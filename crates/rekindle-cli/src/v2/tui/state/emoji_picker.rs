//! Emoji picker state — grid with search, shortcode resolution, cursor navigation.

const EMOJI_GRID: &[(&str, &str)] = &[
    ("\u{1f44d}", "thumbs_up"), ("\u{1f44e}", "thumbs_down"), ("\u{2764}\u{fe0f}", "heart"), ("\u{1f389}", "party"),
    ("\u{1f525}", "fire"), ("\u{1f4af}", "100"), ("\u{1f600}", "grinning"), ("\u{1f602}", "joy"),
    ("\u{1f914}", "thinking"), ("\u{1f440}", "eyes"), ("\u{1f64f}", "pray"), ("\u{2705}", "check"),
    ("\u{274c}", "cross"), ("\u{1f680}", "rocket"), ("\u{1f4a1}", "bulb"), ("\u{1f62d}", "cry"),
    ("\u{1f923}", "rofl"), ("\u{1f60d}", "heart_eyes"), ("\u{1f973}", "party_face"), ("\u{1f60e}", "cool"),
    ("\u{1f91d}", "handshake"), ("\u{1f4aa}", "muscle"), ("\u{1f3c6}", "trophy"), ("\u{2b50}", "star"),
    ("\u{1f4cc}", "pin"), ("\u{1f514}", "bell"), ("\u{1f4ac}", "speech"), ("\u{1f4ce}", "paperclip"),
    ("\u{1f3af}", "bullseye"), ("\u{1f6e0}\u{fe0f}", "tools"), ("\u{26a1}", "zap"), ("\u{1f31f}", "sparkle"),
];

/// Grid columns.
const COLS: usize = 8;

/// Emoji picker with grid cursor, search filter, and shortcode resolution.
#[derive(Debug)]
pub struct EmojiPickerState {
    pub visible: bool,
    /// Index into `filtered`.
    pub selected: usize,
    pub search: String,
    /// Indices into `EMOJI_GRID` matching the current search.
    filtered: Vec<usize>,
    /// The message_id the reaction will target. Set on open, consumed on select.
    pub target_message_id: String,
}

impl EmojiPickerState {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: 0,
            search: String::new(),
            filtered: (0..EMOJI_GRID.len()).collect(),
            target_message_id: String::new(),
        }
    }

    /// Open the picker targeting a specific message for the reaction.
    pub fn open_for_message(&mut self, message_id: String) {
        self.visible = true;
        self.selected = 0;
        self.search.clear();
        self.filtered = (0..EMOJI_GRID.len()).collect();
        self.target_message_id = message_id;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.search.clear();
    }

    pub fn selected_emoji(&self) -> Option<&'static str> {
        self.filtered.get(self.selected).map(|&i| EMOJI_GRID[i].0)
    }

    pub fn filtered_entries(&self) -> Vec<(&'static str, &'static str, bool)> {
        self.filtered.iter().enumerate().map(|(idx, &grid_idx)| {
            let (emoji, name) = EMOJI_GRID[grid_idx];
            (emoji, name, idx == self.selected)
        }).collect()
    }

    pub fn cols(&self) -> usize {
        COLS.min(self.filtered.len().max(1))
    }

    /// Grid cursor navigation with wrapping.
    pub fn navigate(&mut self, dx: isize, dy: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let cols = self.cols();
        let row = self.selected / cols;
        let col = self.selected % cols;
        let new_col = (col as isize + dx).rem_euclid(cols as isize) as usize;
        let rows = (self.filtered.len() + cols - 1) / cols;
        let new_row = (row as isize + dy).rem_euclid(rows as isize) as usize;
        let new_idx = new_row * cols + new_col;
        self.selected = new_idx.min(self.filtered.len().saturating_sub(1));
    }

    /// Type a character into search. Supports `:name:` shortcode syntax —
    /// typing `:fire:` resolves to the matching emoji immediately.
    pub fn type_char(&mut self, c: char) {
        self.search.push(c);
        if self.search.starts_with(':') && self.search.len() > 2 && c == ':' {
            let name = &self.search[1..self.search.len() - 1];
            if let Some(idx) = EMOJI_GRID.iter().position(|(_, n)| *n == name) {
                self.filtered = vec![idx];
                self.selected = 0;
                return;
            }
        }
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.search.pop();
        self.refilter();
    }

    /// Resolve a `:name:` shortcode to an emoji character.
    pub fn resolve_shortcode(input: &str) -> Option<&'static str> {
        if input.starts_with(':') && input.ends_with(':') && input.len() > 2 {
            let name = &input[1..input.len() - 1];
            EMOJI_GRID.iter().find(|(_, n)| *n == name).map(|(emoji, _)| *emoji)
        } else {
            None
        }
    }

    fn refilter(&mut self) {
        let q = self.search.trim_start_matches(':').to_lowercase();
        if q.is_empty() {
            self.filtered = (0..EMOJI_GRID.len()).collect();
        } else {
            self.filtered = EMOJI_GRID.iter().enumerate()
                .filter(|(_, (_, name))| name.contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected = 0;
    }
}

impl Default for EmojiPickerState {
    fn default() -> Self {
        Self::new()
    }
}
