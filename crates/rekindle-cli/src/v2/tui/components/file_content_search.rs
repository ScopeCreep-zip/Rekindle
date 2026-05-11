//! File content search overlay — search inside project files from the TUI.
//!
//! Searches the text content of files in the project tree using fff's
//! SIMD-accelerated literal search, regex engine, and bigram prefilter.
//! No external tools (no grep, no ripgrep) — everything is compiled in.
//!
//! Opened via `Ctrl+G`. Shows a search input at the top and matched
//! lines below with file:line context. Select a result to copy
//! `path:line` to clipboard. Definition matches (fn/struct/class)
//! are tagged.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::super::action::Action;
use super::super::theme::ThemeManager;
use crate::v2::helpers;

/// A single content match for display.
#[derive(Debug, Clone)]
pub struct ContentMatch {
    pub file_path: String,
    pub line_number: u64,
    pub line_content: String,
    pub is_definition: bool,
}

/// File content search overlay state.
pub struct FileContentSearch {
    pub query: String,
    pub results: Vec<ContentMatch>,
    pub list_state: ListState,
    pub visible: bool,
    pub total_files_searched: usize,
    pub total_matches: usize,
}

impl FileContentSearch {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            list_state: ListState::default(),
            visible: false,
            total_files_searched: 0,
            total_matches: 0,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.results.clear();
        self.list_state.select(None);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.results.clear();
    }

    /// Update results from fff content search. Called by the App
    /// after running `search.grep()` on each keystroke.
    pub fn set_results(
        &mut self,
        results: Vec<ContentMatch>,
        total_files_searched: usize,
        total_matches: usize,
    ) {
        self.results = results;
        self.total_files_searched = total_files_searched;
        self.total_matches = total_matches;
        if self.results.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                self.close();
                Some(Action::CloseOverlay)
            }
            KeyCode::Enter => {
                let idx = self.list_state.selected()?;
                let result = self.results.get(idx)?;
                let path = result.file_path.clone();
                #[allow(clippy::cast_possible_truncation)]
                let line = result.line_number as usize;
                self.close();
                Some(Action::ShowFilePreview { path, line: Some(line) })
            }
            KeyCode::Down | KeyCode::Tab => {
                if !self.results.is_empty() {
                    let max = self.results.len() - 1;
                    let i = self.list_state.selected().unwrap_or(0);
                    self.list_state.select(Some((i + 1).min(max)));
                }
                None
            }
            KeyCode::Up | KeyCode::BackTab => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                None // App re-runs content search on the updated query
            }
            KeyCode::Backspace => {
                self.query.pop();
                None
            }
            _ => None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        if !self.visible {
            return;
        }

        let popup = centered_rect(
            area,
            area.width.saturating_sub(8),
            area.height.saturating_sub(4),
        );
        frame.render_widget(Clear, popup);

        let block = Block::bordered()
            .title(" Search File Contents (Ctrl+G) ")
            .border_style(theme.focused_border());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let [input_area, results_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
        ]).areas(inner);

        // Search input with cursor
        let input_line = Line::from(vec![
            Span::raw(format!(" search: {}", self.query)),
            Span::styled("█", Style::new().dim()),
        ]);
        frame.render_widget(Paragraph::new(input_line), input_area);

        // Results
        if self.results.is_empty() {
            let empty = if self.query.is_empty() {
                "  Type to search inside project files..."
            } else {
                "  No matches found."
            };
            frame.render_widget(
                Paragraph::new(empty).style(Style::new().dim()),
                results_area,
            );
            return;
        }

        let items: Vec<ListItem<'_>> = self.results.iter().map(|r| {
            let def_marker = if r.is_definition { " [def]" } else { "" };
            let path_display = helpers::abbreviate_key(&r.file_path);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {path_display}:{}:", r.line_number),
                    Style::new().dim(),
                ),
                Span::raw(format!(
                    " {}",
                    helpers::sanitize_for_display(&r.line_content)
                )),
                Span::styled(def_marker, Style::new().bold()),
            ]))
        }).collect();

        let count_label = format!(
            " {}/{} matches ({} files searched) ",
            self.results.len(),
            self.total_matches,
            self.total_files_searched,
        );
        let list = List::new(items)
            .highlight_style(Style::new().reversed())
            .block(Block::default().title(count_label));

        frame.render_stateful_widget(list, results_area, &mut self.list_state);
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}
