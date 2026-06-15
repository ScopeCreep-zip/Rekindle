//! Emoji picker popup — grid of common emoji with search filter.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Clear};
use ratatui::Frame;

const EMOJI_GRID: &[(&str, &str)] = &[
    ("👍", "thumbs_up"), ("👎", "thumbs_down"), ("❤️", "heart"), ("🎉", "party"),
    ("🔥", "fire"), ("💯", "100"), ("😀", "grinning"), ("😂", "joy"),
    ("🤔", "thinking"), ("👀", "eyes"), ("🙏", "pray"), ("✅", "check"),
    ("❌", "cross"), ("🚀", "rocket"), ("💡", "bulb"), ("😭", "cry"),
    ("🤣", "rofl"), ("😍", "heart_eyes"), ("🥳", "party_face"), ("😎", "cool"),
    ("🤝", "handshake"), ("💪", "muscle"), ("🏆", "trophy"), ("⭐", "star"),
    ("📌", "pin"), ("🔔", "bell"), ("💬", "speech"), ("📎", "paperclip"),
    ("🎯", "bullseye"), ("🛠️", "tools"), ("⚡", "zap"), ("🌟", "sparkle"),
];

const COLS: usize = 8;

pub struct EmojiPicker {
    pub visible: bool,
    pub selected: usize,
    pub search: String,
    filtered: Vec<usize>,
}

impl EmojiPicker {
    pub fn new() -> Self {
        Self {
            visible: false,
            selected: 0,
            search: String::new(),
            filtered: (0..EMOJI_GRID.len()).collect(),
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.selected = 0;
        self.search.clear();
        self.filtered = (0..EMOJI_GRID.len()).collect();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.search.clear();
    }

    pub fn selected_emoji(&self) -> Option<&'static str> {
        self.filtered.get(self.selected).map(|&i| EMOJI_GRID[i].0)
    }

    pub fn navigate(&mut self, dx: isize, dy: isize) {
        if self.filtered.is_empty() { return; }
        let cols = COLS.min(self.filtered.len());
        let row = self.selected / cols;
        let col = self.selected % cols;
        let new_col = (col as isize + dx).rem_euclid(cols as isize) as usize;
        let rows = (self.filtered.len() + cols - 1) / cols;
        let new_row = (row as isize + dy).rem_euclid(rows as isize) as usize;
        let new_idx = new_row * cols + new_col;
        self.selected = new_idx.min(self.filtered.len().saturating_sub(1));
    }

    /// Type a character into the search. Supports both freeform search
    /// and Slack-style `:emoji_name:` syntax — typing `:thumbs_up:` or
    /// `:fire:` resolves to the matching emoji immediately.
    pub fn type_char(&mut self, c: char) {
        self.search.push(c);
        // Check for completed :name: syntax
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
    /// Used by the input box to inline-expand shortcodes typed in messages.
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

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        if !self.visible { return; }

        let width = 34u16;
        let rows = ((self.filtered.len() + COLS - 1) / COLS) as u16;
        let height = (rows + 4).min(12);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let popup = Rect::new(x, y, width, height);

        frame.render_widget(Clear, popup);
        let block = Block::bordered().title(" React — arrows/type to filter, Enter to select, Esc to close ");

        let mut lines: Vec<Line<'_>> = Vec::new();

        // Search line
        lines.push(Line::from(vec![
            Span::raw(" search: "),
            Span::styled(if self.search.is_empty() { "..." } else { &self.search }, Style::new().dim()),
        ]));

        // Emoji grid
        let cols = COLS.min(self.filtered.len().max(1));
        for row_start in (0..self.filtered.len()).step_by(cols) {
            let row_end = (row_start + cols).min(self.filtered.len());
            let spans: Vec<Span<'_>> = (row_start..row_end).map(|idx| {
                let emoji = EMOJI_GRID[self.filtered[idx]].0;
                let style = if idx == self.selected { Style::new().reversed() } else { Style::new() };
                Span::styled(format!(" {emoji} "), style)
            }).collect();
            lines.push(Line::from(spans));
        }

        if self.filtered.is_empty() {
            lines.push(Line::from(Span::styled(" no match", Style::new().dim())));
        }

        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}
