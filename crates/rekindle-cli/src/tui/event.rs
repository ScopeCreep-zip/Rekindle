//! Event types for the TUI event loop.
//!
//! [`Event`] is the unified event type consumed by the App main loop.
//! Events arrive from three sources:
//! 1. Terminal (crossterm) — keyboard, mouse, resize, paste, focus
//! 2. Internal timers — tick (state updates) and render (frame draws)
//! 3. Quit signal — cancellation token or event stream end
//!
//! Subscription events from the daemon (SubscriptionEvent) are consumed
//! directly by App::run() via the dedicated IPC event channel — they do
//! not flow through this Event enum.

/// Unified event type for the TUI terminal event loop.
///
/// Produced by the `Tui` struct's event task and consumed by `App::run()`.
/// Every event is lightweight and cloneable.
#[derive(Clone, Debug)]
#[allow(dead_code)] // Mouse and Paste fields read via pattern binding in event_to_action.
pub enum Event {
    /// Initialization complete — sent once after the event loop starts.
    Init,

    /// State update tick — fires at `tick_rate` Hz (default 4).
    /// Used for: notification expiry, animation advancement, typing indicator cleanup.
    Tick,

    /// Render tick — fires at `frame_rate` Hz (default 30).
    /// Triggers `terminal.draw()` in the App main loop.
    Render,

    /// Keyboard input (Press events only — Release filtered by the event task).
    Key(crossterm::event::KeyEvent),

    /// Mouse input (click, scroll, drag).
    Mouse(crossterm::event::MouseEvent),

    /// Terminal resized to (width, height).
    Resize(u16, u16),

    /// Bracketed paste content.
    Paste(String),

    /// Terminal gained focus (window activated).
    FocusGained,

    /// Terminal lost focus (window deactivated).
    FocusLost,
}
