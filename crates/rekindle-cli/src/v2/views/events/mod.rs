//! Event calendar — list upcoming/past community events, RSVP, create, delete.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use crate::v2::views::{View, ViewQuery};

pub struct EventCalendarView {
    community: String,
    events: Vec<serde_json::Value>,
    selected: usize,
    focus: FocusRing,
    loading: bool,
}

impl EventCalendarView {
    pub fn new(community: String) -> Self {
        Self {
            community,
            events: Vec::new(),
            selected: 0,
            focus: FocusRing::new(vec![FocusId::MessageList]),
            loading: true,
        }
    }

    pub fn community(&self) -> &str { &self.community }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl ViewQuery for EventCalendarView {}

impl View for EventCalendarView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let title = format!(" Events: {} — [r]svp  [d]elete ", self.community);
        let outer = Block::bordered().title(title).border_style(theme.focused_border());

        if self.loading {
            frame.render_widget(
                Paragraph::new("  Loading events...").style(theme.style("dim")).block(outer),
                area,
            );
            return Ok(());
        }

        if self.events.is_empty() {
            frame.render_widget(
                Paragraph::new("  No community events.").style(theme.style("dim")).block(outer),
                area,
            );
            return Ok(());
        }

        let now = Self::now_ms();
        let mut upcoming: Vec<(usize, &serde_json::Value)> = Vec::new();
        let mut past: Vec<(usize, &serde_json::Value)> = Vec::new();
        for (i, ev) in self.events.iter().enumerate() {
            let start = ev.get("start_time").or_else(|| ev.get("startTime"))
                .and_then(|v| v.as_u64()).unwrap_or(0);
            if start > now { upcoming.push((i, ev)); } else { past.push((i, ev)); }
        }

        let upcoming_height = (upcoming.len() as u16 + 2).min(area.height / 2);
        let [upcoming_area, past_area] = Layout::vertical([
            Constraint::Length(upcoming_height),
            Constraint::Min(3),
        ]).areas(area);

        // Upcoming
        let up_block = Block::bordered().title(format!(" Upcoming ({}) ", upcoming.len())).border_style(Style::new().dim());
        let up_items: Vec<ListItem<'_>> = upcoming.iter().map(|(_, ev)| {
            let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let start = ev.get("start_time").or_else(|| ev.get("startTime"))
                .and_then(|v| v.as_u64()).unwrap_or(0);
            let date = if start == 0 { "unknown".to_string() } else { helpers::format_timestamp(start) };
            ListItem::new(Line::from(vec![
                Span::raw(format!("  📅 {date}  ")),
                Span::styled(title, Style::new().bold()),
                Span::styled("  [r]svp [d]elete", Style::new().dim()),
            ]))
        }).collect();
        let mut up_state = ListState::default();
        if !upcoming.is_empty() && self.selected < upcoming.len() {
            up_state.select(Some(self.selected));
        }
        frame.render_stateful_widget(
            List::new(up_items).block(up_block).highlight_style(Style::new().reversed()),
            upcoming_area, &mut up_state,
        );

        // Past
        let past_block = Block::bordered().title(format!(" Past ({}) ", past.len())).border_style(Style::new().dim());
        let past_items: Vec<ListItem<'_>> = past.iter().map(|(_, ev)| {
            let title = ev.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let start = ev.get("start_time").or_else(|| ev.get("startTime"))
                .and_then(|v| v.as_u64()).unwrap_or(0);
            let date = if start == 0 { "unknown".to_string() } else { helpers::format_timestamp(start) };
            ListItem::new(Line::from(vec![
                Span::raw(format!("  📅 {date}  ")),
                Span::styled(title, Style::new().dim()),
            ]))
        }).collect();
        frame.render_widget(List::new(past_items).block(past_block), past_area);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) => {
                if self.selected + 1 < self.events.len() { self.selected += 1; }
                Ok(None)
            }
            Action::ScrollUp(_) => {
                self.selected = self.selected.saturating_sub(1);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<Option<Action>> {
        if let CommandResult::EventsLoaded { events } = result {
            self.events = events;
            self.loading = false;
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected + 1 < self.events.len() { self.selected += 1; }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
