//! Ephemeral state — connection-scoped, cleared on reconnect.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum TypingKey {
    Channel {
        community: String,
        channel: String,
        pseudonym: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: Instant,
}

const TOAST_AUTO_DISMISS: Duration = Duration::from_secs(4);
const TOAST_DEDUP_WINDOW: Duration = Duration::from_secs(2);
const TOAST_MAX_VISIBLE: usize = 3;

/// Bounded toast stack. Deduplicates within 2s window. Maximum 3 visible.
/// Info/Success auto-dismiss at 4s. Warning/Error persist until dismissed.
#[derive(Debug)]
pub struct ToastStack {
    pub toasts: VecDeque<Toast>,
}

impl ToastStack {
    pub fn new() -> Self {
        Self { toasts: VecDeque::new() }
    }

    pub fn push(&mut self, message: String, level: ToastLevel, now: Instant) {
        let dominated = self.toasts.iter().any(|t| {
            t.message == message && now.duration_since(t.created_at) < TOAST_DEDUP_WINDOW
        });
        if dominated {
            return;
        }
        while self.toasts.len() >= TOAST_MAX_VISIBLE {
            self.toasts.pop_back();
        }
        self.toasts.push_front(Toast { message, level, created_at: now });
    }

    pub fn tick(&mut self, now: Instant) {
        self.toasts.retain(|t| match t.level {
            ToastLevel::Info | ToastLevel::Success => {
                now.duration_since(t.created_at) < TOAST_AUTO_DISMISS
            }
            ToastLevel::Warning | ToastLevel::Error => true,
        });
    }

    pub fn dismiss_oldest(&mut self) {
        self.toasts.pop_back();
    }

    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }
}

impl Default for ToastStack {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalScope {
    System,
    Community,
    Channel,
}

/// Ordered by visual weight. Higher values render further left.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SignalPriority {
    Info = 0,
    Warning = 1,
    Critical = 2,
}

/// Persistent notification rail signal. Non-dismissible signals persist
/// until state-driven removal.
#[derive(Clone, Debug)]
pub struct RailSignal {
    pub id: String,
    pub scope: SignalScope,
    pub text: String,
    pub priority: SignalPriority,
    pub dismissible: bool,
}

const MAX_DISMISSED: usize = 128;

/// Two-rail notification system. Community/channel signals render below
/// the tab bar; system signals render above the status bar on dashboard.
/// Dismissed IDs retained with FIFO eviction at 128 entries.
#[derive(Debug)]
pub struct NotificationRails {
    signals: Vec<RailSignal>,
    /// FIFO eviction at 128 entries. O(1) pop_front via VecDeque.
    dismissed: VecDeque<String>,
}

impl NotificationRails {
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            dismissed: VecDeque::new(),
        }
    }

    /// Insert or update. Silently ignores dismissed IDs.
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

    /// State-driven removal. Does NOT add to dismissed set.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.signals.len();
        self.signals.retain(|s| s.id != id);
        self.signals.len() < before
    }

    /// User-initiated dismiss. Adds to dismissed set. No-op on non-dismissible.
    #[must_use = "returns whether a signal was dismissed"]
    pub fn dismiss(&mut self, id: &str) -> bool {
        let is_dismissible = self.signals.iter().any(|s| s.id == id && s.dismissible);
        if is_dismissible {
            while self.dismissed.len() >= MAX_DISMISSED {
                self.dismissed.pop_front();
            }
            if !self.dismissed.iter().any(|d| d == id) {
                self.dismissed.push_back(id.to_string());
            }
            self.signals.retain(|s| s.id != id);
            true
        } else {
            false
        }
    }

    /// Dismiss the highest-priority dismissible signal.
    #[must_use = "returns whether a signal was dismissed"]
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

    pub fn clear_scope(&mut self, scope: SignalScope) {
        self.signals.retain(|s| s.scope != scope);
    }

    /// Remove signals whose ID contains the given community key.
    pub fn clear_for_community(&mut self, community_id: &str) {
        self.signals.retain(|s| !s.id.contains(community_id));
    }

    pub fn has_community_signals(&self) -> bool {
        self.signals.iter().any(|s| matches!(s.scope, SignalScope::Community | SignalScope::Channel))
    }

    pub fn has_system_signals(&self) -> bool {
        self.signals.iter().any(|s| s.scope == SignalScope::System)
    }

    /// 0 or 1.
    pub fn community_rail_height(&self) -> u16 {
        u16::from(self.has_community_signals())
    }

    /// 0 or 1.
    pub fn system_rail_height(&self) -> u16 {
        u16::from(self.has_system_signals())
    }

    /// Sorted by priority descending.
    pub fn community_signals(&self) -> Vec<&RailSignal> {
        let mut sigs: Vec<&RailSignal> = self.signals.iter()
            .filter(|s| matches!(s.scope, SignalScope::Community | SignalScope::Channel))
            .collect();
        sigs.sort_by(|a, b| b.priority.cmp(&a.priority));
        sigs
    }

    /// Sorted by priority descending.
    pub fn system_signals(&self) -> Vec<&RailSignal> {
        let mut sigs: Vec<&RailSignal> = self.signals.iter()
            .filter(|s| s.scope == SignalScope::System)
            .collect();
        sigs.sort_by(|a, b| b.priority.cmp(&a.priority));
        sigs
    }
}

impl Default for NotificationRails {
    fn default() -> Self {
        Self::new()
    }
}

const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_FRAMES: &[&str] = &["|", "/", "-", "\\"];

#[derive(Debug)]
pub struct SpinnerState {
    pub frame: usize,
    pub animated: bool,
    pub unicode: bool,
    pub label: String,
    pub active: bool,
}

impl SpinnerState {
    pub fn new(animated: bool, unicode: bool) -> Self {
        Self { frame: 0, animated, unicode, label: String::new(), active: false }
    }

    pub fn set_label(&mut self, label: impl Into<String>) { self.label = label.into(); }
    pub fn start(&mut self) { self.active = true; self.frame = 0; }
    pub fn stop(&mut self) { self.active = false; }
    pub fn is_active(&self) -> bool { self.active }

    pub fn tick(&mut self) {
        if self.active {
            self.frame = self.frame.wrapping_add(1);
        }
    }

    pub fn glyph(&self) -> &str {
        if !self.animated {
            return "[...]";
        }
        let frames = if self.unicode { BRAILLE_FRAMES } else { ASCII_FRAMES };
        frames[self.frame % frames.len()]
    }
}

#[derive(Clone, Debug)]
pub struct IdentitySnapshot {
    pub public_key: String,
    pub display_name: String,
}

#[derive(Debug)]
pub struct EphemeralState {
    /// Expired at 5s by deadline check.
    pub typing_indicators: HashMap<TypingKey, Instant>,
    pub rails: NotificationRails,
    pub toasts: ToastStack,
    pub spinner: SpinnerState,
    /// 30s after copy.
    pub clipboard_clear_at: Option<Instant>,
    /// Reserved for future event replay. Not written today.
    pub last_event_seq: Option<u64>,
    pub idle_frames: u32,
    pub last_input_at: Instant,
    pub identity: Option<IdentitySnapshot>,
    /// Peer count samples for dashboard sparkline. Capped at 60.
    pub peer_history: VecDeque<f64>,
    /// Community count samples for dashboard area graph. Capped at 60.
    pub community_history: VecDeque<f64>,
    /// 0=Members, 1=Bans, 2=Queue.
    pub moderation_tab: u8,
    pub moderation_selected: usize,
    pub invite_selected: usize,
    pub events_selected: usize,
    /// Persistent offset — not reset per frame. Accumulates from j/k input.
    pub file_preview_scroll_offset: i32,
    pub onboarding_step: usize,
}

impl EphemeralState {
    pub fn new(animated: bool, unicode: bool, now: Instant) -> Self {
        Self {
            typing_indicators: HashMap::new(),
            rails: NotificationRails::new(),
            toasts: ToastStack::new(),
            spinner: SpinnerState::new(animated, unicode),
            clipboard_clear_at: None,
            last_event_seq: None,
            idle_frames: 0,
            last_input_at: now,
            identity: None,
            peer_history: VecDeque::new(),
            community_history: VecDeque::new(),
            moderation_tab: 0,
            moderation_selected: 0,
            invite_selected: 0,
            events_selected: 0,
            file_preview_scroll_offset: 0,
            onboarding_step: 0,
        }
    }

    /// Clears typing indicators and event sequence. Rails and toasts
    /// are preserved across reconnect.
    pub fn clear_connection_scoped(&mut self) {
        self.typing_indicators.clear();
        self.last_event_seq = None;
        self.moderation_tab = 0;
        self.moderation_selected = 0;
        self.invite_selected = 0;
        self.events_selected = 0;
        self.file_preview_scroll_offset = 0;
        self.onboarding_step = 0;
    }
}
