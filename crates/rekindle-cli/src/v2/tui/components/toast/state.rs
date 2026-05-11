//! Toast state — bounded stack with dedup and timed auto-dismiss.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::super::super::action::ToastLevel;

const AUTO_DISMISS: Duration = Duration::from_secs(4);
const DEDUP_WINDOW: Duration = Duration::from_secs(2);
const MAX_VISIBLE: usize = 3;

pub(super) struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: Instant,
}

/// Bounded notification stack with dedup and timed auto-dismiss.
pub struct NotificationStack {
    pub(super) toasts: VecDeque<Toast>,
}

impl NotificationStack {
    pub fn new() -> Self { Self { toasts: VecDeque::new() } }

    pub fn push(&mut self, message: String, level: ToastLevel) {
        let dominated = self.toasts.iter().any(|t| {
            t.message == message && t.created_at.elapsed() < DEDUP_WINDOW
        });
        if dominated { return; }
        while self.toasts.len() >= MAX_VISIBLE { self.toasts.pop_back(); }
        self.toasts.push_front(Toast { message, level, created_at: Instant::now() });
    }

    pub fn tick(&mut self) {
        self.toasts.retain(|t| match t.level {
            ToastLevel::Info | ToastLevel::Success => t.created_at.elapsed() < AUTO_DISMISS,
            ToastLevel::Warning | ToastLevel::Error => true,
        });
    }

    pub fn dismiss_oldest(&mut self) { self.toasts.pop_back(); }
    pub fn is_empty(&self) -> bool { self.toasts.is_empty() }
}
