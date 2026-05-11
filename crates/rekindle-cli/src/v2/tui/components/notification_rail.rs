//! Notification rail — scope-partitioned persistent signal surfaces.
//!
//! Two independent rails:
//! - `CommunityRail` — top, below tab bar, community/channel operator signals
//! - `SystemRail` — bottom, above status bar, dashboard system duress signals
//!
//! Both collapse to zero height when empty. Signals are managed by the
//! App reducer — added on subscription events, removed on state change
//! or explicit dismiss.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::super::theme::ThemeManager;

/// A single signal in the notification rail.
#[derive(Debug, Clone)]
pub struct RailSignal {
    /// Unique identifier for dedup and dismiss.
    pub id: String,
    /// The signal scope — determines which rail it appears in.
    pub scope: SignalScope,
    /// Display text.
    pub text: String,
    /// Visual priority — higher values render further left (more prominent).
    pub priority: SignalPriority,
    /// Whether the user can dismiss this signal (Esc/click).
    pub dismissible: bool,
}

/// Signal scope — determines which rail surface carries the signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalScope {
    /// System-level: daemon health, network, search index. Dashboard only.
    System,
    /// Community-level: raid alert, announcements, upcoming events.
    Community,
    /// Channel-level: lockdown, slow mode.
    Channel,
}

/// Visual priority for ordering signals within a rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignalPriority {
    /// Informational — low visual weight (dim).
    Info = 0,
    /// Warning — medium visual weight (yellow).
    Warning = 1,
    /// Critical — high visual weight (red, bold).
    Critical = 2,
}

/// Manages signals for both rail surfaces.
pub struct NotificationRails {
    signals: Vec<RailSignal>,
    /// IDs of signals the user has explicitly dismissed. Prevents
    /// dismissed announcements from reappearing on the next
    /// subscription event delivery. Uses Vec for deterministic FIFO
    /// eviction order (oldest dismissed entries removed first when at capacity).
    dismissed: Vec<String>,
}

impl NotificationRails {
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            dismissed: Vec::new(),
        }
    }

    /// Add or update a signal. Silently ignores signals the user has dismissed.
    pub fn set(&mut self, signal: RailSignal) {
        if self.dismissed.iter().any(|d| d == &signal.id) {
            return;
        }
        if let Some(existing) = self.signals.iter_mut().find(|s| s.id == signal.id) {
            *existing = signal;
        } else {
            self.signals.push(signal);
        }
    }

    /// Remove a signal by id (state-driven removal, e.g., raid alert cleared).
    /// Does NOT add to dismissed set — the signal can reappear if re-emitted.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.signals.len();
        self.signals.retain(|s| s.id != id);
        self.signals.len() < before
    }

    /// Dismiss a signal by id (user-initiated). Adds to dismissed list
    /// so it won't reappear on the next subscription event delivery.
    /// Only works on dismissible signals — non-dismissible signals are ignored.
    /// Dismissed list is capped at 128 entries with FIFO eviction (oldest first).
    pub fn dismiss(&mut self, id: &str) -> bool {
        const MAX_DISMISSED: usize = 128;
        let is_dismissible = self.signals.iter().any(|s| s.id == id && s.dismissible);
        if is_dismissible {
            // FIFO eviction — remove from the front (oldest) when at capacity
            while self.dismissed.len() >= MAX_DISMISSED {
                self.dismissed.remove(0);
            }
            if !self.dismissed.iter().any(|d| d == id) {
                self.dismissed.push(id.to_string());
            }
            self.signals.retain(|s| s.id != id);
            true
        } else {
            false
        }
    }

    /// Dismiss the highest-priority dismissible signal. Returns true if
    /// a signal was dismissed. Used by the Cancel/Escape key handler as
    /// a fallback when no overlay, search, or input mode is active.
    pub fn dismiss_first_dismissible(&mut self) -> bool {
        let id = self.signals.iter()
            .filter(|s| s.dismissible)
            .max_by_key(|s| s.priority)
            .map(|s| s.id.clone());
        if let Some(id) = id {
            self.dismiss(&id)
        } else {
            false
        }
    }

    /// Remove all signals matching a scope.
    pub fn clear_scope(&mut self, scope: SignalScope) {
        self.signals.retain(|s| s.scope != scope);
    }

    /// Whether there are any signals for the community/channel rail.
    pub fn has_community_signals(&self) -> bool {
        self.signals.iter().any(|s| matches!(s.scope, SignalScope::Community | SignalScope::Channel))
    }

    /// Whether there are any signals for the system rail (dashboard).
    pub fn has_system_signals(&self) -> bool {
        self.signals.iter().any(|s| s.scope == SignalScope::System)
    }

    /// Height of the community rail (0 or 1).
    pub fn community_rail_height(&self) -> u16 {
        u16::from(self.has_community_signals())
    }

    /// Height of the system rail (0 or 1).
    pub fn system_rail_height(&self) -> u16 {
        u16::from(self.has_system_signals())
    }

    /// Render the community/channel rail (top position).
    pub fn render_community_rail(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let signals: Vec<&RailSignal> = self.signals.iter()
            .filter(|s| matches!(s.scope, SignalScope::Community | SignalScope::Channel))
            .collect();

        if signals.is_empty() {
            return;
        }

        let spans = build_rail_spans(&signals, theme);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// Render the system rail (bottom position, dashboard only).
    pub fn render_system_rail(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let signals: Vec<&RailSignal> = self.signals.iter()
            .filter(|s| s.scope == SignalScope::System)
            .collect();

        if signals.is_empty() {
            return;
        }

        let spans = build_rail_spans(&signals, theme);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }
}

/// Build styled spans from a sorted signal list.
fn build_rail_spans<'a>(signals: &[&RailSignal], theme: &'a ThemeManager) -> Vec<Span<'a>> {
    let mut sorted: Vec<&&RailSignal> = signals.iter().collect();
    sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

    let mut spans = Vec::new();
    for (i, signal) in sorted.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::new().dim()));
        }

        let (icon, style) = match signal.priority {
            SignalPriority::Critical => ("🔴 ", Style::new().fg(theme.color("error")).bold()),
            SignalPriority::Warning => ("⚠ ", Style::new().fg(theme.color("warning"))),
            SignalPriority::Info => ("ℹ ", Style::new().fg(theme.color("info"))),
        };

        spans.push(Span::raw(" "));
        spans.push(Span::styled(icon, style));
        spans.push(Span::styled(signal.text.clone(), style));
    }

    spans
}
