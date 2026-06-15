//! Transfer progress rail — renders active bulk transfers inline.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub transfer_id: String,
    pub direction: String,
    pub filename: String,
    pub bytes_transferred: u64,
    pub total_size: u64,
    pub status: TransferStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Active,
    Completed,
    Failed(String),
    Cancelled,
}

pub struct TransferProgressRail {
    pub transfers: Vec<TransferProgress>,
}

impl TransferProgressRail {
    pub fn new() -> Self {
        Self { transfers: Vec::new() }
    }

    pub fn visible(&self) -> bool {
        !self.transfers.is_empty()
    }

    pub fn height(&self) -> u16 {
        if self.transfers.is_empty() { 0 } else { (self.transfers.len() as u16 + 1).min(5) }
    }

    pub fn update(&mut self, transfer_id: &str, bytes: u64, total: u64) {
        if let Some(t) = self.transfers.iter_mut().find(|t| t.transfer_id == transfer_id) {
            t.bytes_transferred = bytes;
            t.total_size = total;
        } else {
            self.transfers.push(TransferProgress {
                transfer_id: transfer_id.to_string(),
                direction: "↕".into(),
                filename: transfer_id[..12.min(transfer_id.len())].to_string(),
                bytes_transferred: bytes,
                total_size: total,
                status: TransferStatus::Active,
            });
        }
    }

    pub fn complete(&mut self, transfer_id: &str) {
        if let Some(t) = self.transfers.iter_mut().find(|t| t.transfer_id == transfer_id) {
            t.status = TransferStatus::Completed;
            t.bytes_transferred = t.total_size;
        }
    }

    pub fn fail(&mut self, transfer_id: &str, reason: &str) {
        if let Some(t) = self.transfers.iter_mut().find(|t| t.transfer_id == transfer_id) {
            t.status = TransferStatus::Failed(reason.to_string());
        }
    }

    pub fn gc_completed(&mut self) {
        self.transfers.retain(|t| t.status == TransferStatus::Active);
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        if self.transfers.is_empty() { return; }

        let block = Block::bordered().title(" 📦 Transfers ").border_style(Style::new().dim());
        let lines: Vec<Line<'_>> = self.transfers.iter().map(|t| {
            let pct = if t.total_size > 0 { (t.bytes_transferred * 100 / t.total_size) as u16 } else { 0 };
            let bar_width = 16u16;
            let filled = (bar_width * pct / 100).min(bar_width);
            let bar: String = "█".repeat(filled as usize) + &"░".repeat((bar_width - filled) as usize);
            let size = format_bytes(t.bytes_transferred);
            let total = format_bytes(t.total_size);

            let (status_glyph, status_label) = match &t.status {
                TransferStatus::Active => ("", ""),
                TransferStatus::Completed => (" DONE", " [DONE]"),
                TransferStatus::Failed(_) => (" FAILED", " [FAILED]"),
                TransferStatus::Cancelled => (" CANCELLED", " [CANCELLED]"),
            };

            Line::from(vec![
                Span::raw(format!("  {} {} ", t.direction, t.filename)),
                Span::raw(bar),
                Span::styled(format!("  {pct}%  {size}/{total}"), Style::new().dim()),
                Span::styled(format!("{status_glyph}{status_label}"), if t.status == TransferStatus::Completed {
                    Style::new().bold()
                } else if matches!(t.status, TransferStatus::Failed(_)) {
                    Style::new().bold()
                } else {
                    Style::new()
                }),
            ])
        }).collect();

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

fn format_bytes(b: u64) -> String {
    if b < 1024 { format!("{b}B") }
    else if b < 1024 * 1024 { format!("{:.1}KB", b as f64 / 1024.0) }
    else if b < 1024 * 1024 * 1024 { format!("{:.1}MB", b as f64 / (1024.0 * 1024.0)) }
    else { format!("{:.1}GB", b as f64 / (1024.0 * 1024.0 * 1024.0)) }
}
