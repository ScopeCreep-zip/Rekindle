//! Toast notification stack.
//!
//! Manages a bounded stack of timed notifications. Renders in the
//! bottom-right corner of the terminal, stacked upward.
//!
//! Rules:
//! - Max 3 visible simultaneously
//! - Info/Success auto-dismiss after 4 seconds
//! - Warning/Error require explicit Esc to dismiss
//! - Identical messages within 2 seconds are deduplicated
//! - Every toast displays a text label ([INFO]/[OK]/[WARN]/[ERR]) AND
//!   a colored border — color is never the sole indicator
//!
//! Source patterns:
//! - vortix `ui/overlays/toast.rs` — anchor, border, level styling
//! - oxicord `presentation/ui/notification_popup.rs` — transient toasts

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::action::ToastLevel;
use crate::tui::theme::ThemeManager;

/// Auto-dismiss duration for Info and Success toasts.
const AUTO_DISMISS: Duration = Duration::from_secs(4);

/// Deduplication window — identical messages within this window are suppressed.
const DEDUP_WINDOW: Duration = Duration::from_secs(2);

/// Maximum number of simultaneously visible toasts.
const MAX_VISIBLE: usize = 3;

/// A single toast notification.
struct Toast {
    message: String,
    level: ToastLevel,
    created_at: Instant,
}

/// Bounded notification stack with dedup and timed auto-dismiss.
pub struct NotificationStack {
    toasts: VecDeque<Toast>,
}

impl NotificationStack {
    /// Create an empty notification stack.
    pub fn new() -> Self {
        Self {
            toasts: VecDeque::new(),
        }
    }

    /// Push a new notification.
    ///
    /// Deduplicates: if an identical message exists within the dedup window,
    /// the new notification is silently dropped. Evicts the oldest toast when
    /// the stack exceeds `MAX_VISIBLE`.
    pub fn push(&mut self, message: String, level: ToastLevel) {
        // Dedup: skip if identical message exists within window
        let dominated = self.toasts.iter().any(|t| {
            t.message == message && t.created_at.elapsed() < DEDUP_WINDOW
        });
        if dominated {
            return;
        }

        // Evict oldest to stay within capacity
        while self.toasts.len() >= MAX_VISIBLE {
            self.toasts.pop_back();
        }

        self.toasts.push_front(Toast {
            message,
            level,
            created_at: Instant::now(),
        });
    }

    /// Advance time — removes expired auto-dismiss toasts.
    ///
    /// Called on every Tick event (4 Hz default).
    pub fn tick(&mut self) {
        self.toasts.retain(|t| match t.level {
            ToastLevel::Info | ToastLevel::Success => t.created_at.elapsed() < AUTO_DISMISS,
            ToastLevel::Warning | ToastLevel::Error => true,
        });
    }

    /// Dismiss the oldest (bottom) toast. Called on Esc when toasts are visible.
    pub fn dismiss_oldest(&mut self) {
        self.toasts.pop_back();
    }

    /// Whether any toasts are visible.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// Render the toast stack in the top-right corner, stacking downward.
    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        for (i, toast) in self.toasts.iter().enumerate() {
            let width = 40u16.min(area.width.saturating_sub(4));
            let height = 3u16;
            let x = area.right().saturating_sub(width + 2);

            // Stack downward from top-right: first toast at y=1, second at y=5, etc.
            #[allow(clippy::cast_possible_truncation)]
            let y = area.y + 1 + (i as u16) * (height + 1);

            let toast_area = Rect { x, y, width, height };
            if y + height > area.bottom() {
                break; // off screen
            }

            // Level determines border color AND text label — never color alone
            let (border_color, label) = match toast.level {
                ToastLevel::Info => (theme.color("info"), "[INFO]"),
                ToastLevel::Success => (theme.color("success"), "[OK]"),
                ToastLevel::Warning => (theme.color("warning"), "[WARN]"),
                ToastLevel::Error => (theme.color("error"), "[ERR]"),
            };

            frame.render_widget(Clear, toast_area);
            let block = Block::bordered()
                .title(format!(" {label} "))
                .border_style(Style::default().fg(border_color));
            let para = Paragraph::new(toast.message.as_str())
                .block(block)
                .wrap(Wrap { trim: true });
            frame.render_widget(para, toast_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_count() {
        let mut stack = NotificationStack::new();
        stack.push("a".into(), ToastLevel::Info);
        stack.push("b".into(), ToastLevel::Info);
        assert_eq!(stack.toasts.len(), 2);
    }

    #[test]
    fn dedup_identical_within_window() {
        let mut stack = NotificationStack::new();
        stack.push("hello".into(), ToastLevel::Info);
        stack.push("hello".into(), ToastLevel::Info);
        assert_eq!(stack.toasts.len(), 1);
    }

    #[test]
    fn no_dedup_different_messages() {
        let mut stack = NotificationStack::new();
        stack.push("a".into(), ToastLevel::Info);
        stack.push("b".into(), ToastLevel::Info);
        assert_eq!(stack.toasts.len(), 2);
    }

    #[test]
    fn evicts_oldest_at_capacity() {
        let mut stack = NotificationStack::new();
        stack.push("a".into(), ToastLevel::Error);
        stack.push("b".into(), ToastLevel::Error);
        stack.push("c".into(), ToastLevel::Error);
        stack.push("d".into(), ToastLevel::Error);
        assert_eq!(stack.toasts.len(), MAX_VISIBLE);
        // Newest should be at front
        assert_eq!(stack.toasts.front().unwrap().message, "d");
    }

    #[test]
    fn tick_removes_expired_info() {
        let mut stack = NotificationStack::new();
        stack.toasts.push_front(Toast {
            message: "old".into(),
            level: ToastLevel::Info,
            created_at: Instant::now() - Duration::from_secs(10),
        });
        stack.toasts.push_front(Toast {
            message: "new".into(),
            level: ToastLevel::Info,
            created_at: Instant::now(),
        });
        stack.tick();
        assert_eq!(stack.toasts.len(), 1);
        assert_eq!(stack.toasts.front().unwrap().message, "new");
    }

    #[test]
    fn tick_keeps_sticky_errors() {
        let mut stack = NotificationStack::new();
        stack.toasts.push_front(Toast {
            message: "error".into(),
            level: ToastLevel::Error,
            created_at: Instant::now() - Duration::from_secs(60),
        });
        stack.tick();
        assert_eq!(stack.toasts.len(), 1);
    }

    #[test]
    fn tick_keeps_sticky_warnings() {
        let mut stack = NotificationStack::new();
        stack.toasts.push_front(Toast {
            message: "warn".into(),
            level: ToastLevel::Warning,
            created_at: Instant::now() - Duration::from_secs(60),
        });
        stack.tick();
        assert_eq!(stack.toasts.len(), 1);
    }

    #[test]
    fn dismiss_oldest_removes_bottom() {
        let mut stack = NotificationStack::new();
        stack.push("first".into(), ToastLevel::Error);
        stack.push("second".into(), ToastLevel::Error);
        stack.dismiss_oldest();
        assert_eq!(stack.toasts.len(), 1);
        assert_eq!(stack.toasts.front().unwrap().message, "second");
    }

    #[test]
    fn is_empty_works() {
        let mut stack = NotificationStack::new();
        assert!(stack.is_empty());
        stack.push("a".into(), ToastLevel::Info);
        assert!(!stack.is_empty());
    }
}
