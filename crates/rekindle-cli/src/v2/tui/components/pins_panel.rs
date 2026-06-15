//! Pins panel — sidebar showing pinned messages for the current channel.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub struct PinsPanel {
    pub visible: bool,
    pub pins: Vec<PinDisplay>,
    pub selected: usize,
    pub community: String,
    pub channel: String,
}

#[derive(Debug, Clone)]
pub struct PinDisplay {
    pub message_id: String,
    pub channel_id: String,
    pub pinned_by: String,
    pub pinned_at: u64,
    pub body_preview: String,
}

impl PinsPanel {
    pub fn new() -> Self {
        Self {
            visible: false,
            pins: Vec::new(),
            selected: 0,
            community: String::new(),
            channel: String::new(),
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn set_pins(&mut self, pins: Vec<PinDisplay>) {
        self.pins = pins;
        self.selected = 0;
    }

    pub fn scroll_up(&mut self) { self.selected = self.selected.saturating_sub(1); }
    pub fn scroll_down(&mut self) {
        if self.selected + 1 < self.pins.len() { self.selected += 1; }
    }

    pub fn selected_pin(&self) -> Option<&PinDisplay> {
        self.pins.get(self.selected)
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        if !self.visible { return; }

        let title = format!(" 📌 Pinned ({}) — [u]npin  [p] close ", self.pins.len());
        let block = Block::bordered().title(title).border_style(Style::new().dim());

        if self.pins.is_empty() {
            frame.render_widget(
                Paragraph::new("  No pinned messages.").style(Style::new().dim()).block(block),
                area,
            );
            return;
        }

        let items: Vec<ListItem<'_>> = self.pins.iter().map(|p| {
            let preview = if p.body_preview.len() > 60 {
                format!("{}...", &p.body_preview[..57])
            } else {
                p.body_preview.clone()
            };
            let by_short = if p.pinned_by.len() > 12 {
                format!("{}...", &p.pinned_by[..9])
            } else {
                p.pinned_by.clone()
            };
            let age = if p.pinned_at == 0 {
                String::new()
            } else {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let elapsed_secs = now_ms.saturating_sub(p.pinned_at) / 1000;
                if elapsed_secs < 60 { format!(" — {elapsed_secs}s ago") }
                else if elapsed_secs < 3600 { format!(" — {}m ago", elapsed_secs / 60) }
                else if elapsed_secs < 86400 { format!(" — {}h ago", elapsed_secs / 3600) }
                else { format!(" — {}d ago", elapsed_secs / 86400) }
            };
            ListItem::new(vec![
                Line::from(Span::raw(format!("  {preview}"))),
                Line::from(vec![
                    Span::styled(format!("    pinned by {by_short}{age}"), Style::new().dim()),
                ]),
            ])
        }).collect();

        let mut state = ListState::default();
        state.select(Some(self.selected.min(items.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items).block(block).highlight_style(Style::new().reversed()),
            area, &mut state,
        );
    }
}
