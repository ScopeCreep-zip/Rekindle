//! File preview view — read-only display of file contents with line numbers.
//!
//! Opened when a user selects a file from the quick switcher (in non-input
//! mode), clicks a file path in a message, or selects a content search
//! result. Shows the file contents with line numbers, scrollable, and
//! optionally jumps to a specific line.
//!
//! Reads the file directly from the filesystem — no daemon IPC needed.
//! File paths are resolved relative to cwd (the project root).
//!
//! Controls: j/k scroll, G jump to bottom, gg jump to top, q/Esc close,
//! y yank current line to clipboard.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use super::{View, ViewQuery};

/// File preview view state.
pub struct FilePreviewView {
    focus: FocusRing,
    /// Relative path of the file being previewed.
    file_path: String,
    /// File contents split into lines.
    lines: Vec<String>,
    /// Scroll/selection state.
    list_state: ListState,
    /// Optional target line to jump to on first render.
    target_line: Option<usize>,
    /// Whether the file was loaded successfully.
    loaded: bool,
    /// Error message if the file couldn't be read.
    error: Option<String>,
}

impl FilePreviewView {
    pub fn new(file_path: String, target_line: Option<usize>) -> Self {
        let mut view = Self {
            focus: FocusRing::new(vec![FocusId::MessageList]),
            file_path,
            lines: Vec::new(),
            list_state: ListState::default(),
            target_line,
            loaded: false,
            error: None,
        };
        view.load_file();
        view
    }

    pub fn file_path(&self) -> &str {
        &self.file_path
    }

    fn load_file(&mut self) {
        let path = std::path::Path::new(&self.file_path);
        // Try relative to cwd first, then absolute
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
        };

        // Reject files larger than 10MB to prevent memory exhaustion
        const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
        match std::fs::metadata(&resolved) {
            Ok(meta) if meta.len() > MAX_FILE_SIZE => {
                self.error = Some(format!(
                    "File too large ({}) — maximum preview size is 10MB",
                    crate::v2::helpers::format_bytes(meta.len())
                ));
                self.loaded = true;
                return;
            }
            Ok(meta) if !meta.is_file() => {
                self.error = Some(format!("{} is not a regular file", resolved.display()));
                self.loaded = true;
                return;
            }
            Err(e) => {
                self.error = Some(format!("Cannot read {}: {e}", resolved.display()));
                self.loaded = true;
                return;
            }
            _ => {}
        }

        match std::fs::read_to_string(&resolved) {
            Ok(content) => {
                self.lines = content.lines().map(str::to_string).collect();
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.loaded = true;

                // Jump to target line if specified
                if let Some(target) = self.target_line {
                    let line_idx = target.saturating_sub(1).min(self.lines.len().saturating_sub(1));
                    self.list_state.select(Some(line_idx));
                } else {
                    self.list_state.select(Some(0));
                }
            }
            Err(e) => {
                self.error = Some(format!("Cannot read {}: {e}", resolved.display()));
                self.loaded = true;
            }
        }
    }

    fn build_items(&self) -> Vec<ListItem<'static>> {
        let gutter_width = self.lines.len().to_string().len();

        self.lines.iter().enumerate().map(|(i, line)| {
            let line_num = i + 1;
            let sanitized = helpers::sanitize_for_display(line);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {line_num:>gutter_width$} │ "),
                    Style::new().dim(),
                ),
                Span::raw(sanitized),
            ]))
        }).collect()
    }
}

impl ViewQuery for FilePreviewView {}

impl View for FilePreviewView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let [content_area, help_area] = Layout::vertical([
            Constraint::Fill(1), Constraint::Length(1),
        ]).areas(area);

        let title = format!(" {} ({} lines) ", self.file_path, self.lines.len());
        let block = Block::bordered().title(title).border_style(theme.focused_border());

        if let Some(ref error) = self.error {
            frame.render_widget(
                Paragraph::new(format!("  {error}"))
                    .style(theme.style("error"))
                    .block(block),
                content_area,
            );
        } else if self.lines.is_empty() {
            frame.render_widget(
                Paragraph::new("  (empty file)").style(theme.style("dim")).block(block),
                content_area,
            );
        } else {
            let items = self.build_items();
            frame.render_stateful_widget(
                List::new(items).block(block).highlight_style(Style::new().reversed()),
                content_area, &mut self.list_state,
            );
        }

        // Help bar
        frame.render_widget(Paragraph::new(Line::from(
            Span::styled(
                "  j/k scroll  G bottom  gg top  y yank line  q/Esc close",
                Style::new().dim(),
            ),
        )), help_area);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) => {
                let max = self.lines.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
            }
            Action::ScrollUp(_) => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
            }
            Action::ScrollToBottom => {
                let max = self.lines.len().saturating_sub(1);
                self.list_state.select(Some(max));
            }
            Action::ScrollToTop => {
                self.list_state.select(Some(0));
            }
            Action::ScrollPageDown => {
                let max = self.lines.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 20).min(max)));
            }
            Action::ScrollPageUp => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(20)));
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.lines.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Char('G') | KeyCode::End => {
                let max = self.lines.len().saturating_sub(1);
                self.list_state.select(Some(max));
                None
            }
            KeyCode::Home => {
                self.list_state.select(Some(0));
                None
            }
            KeyCode::Char('y') => {
                // Yank current line to clipboard
                let idx = self.list_state.selected().unwrap_or(0);
                self.lines.get(idx).map(|line| Action::YankToClipboard { text: line.clone() })
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                Some(Action::Back)
            }
            _ => None,
        }
    }

    fn on_command_result(&mut self, _result: CommandResult) -> Result<()> { Ok(()) }
    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
