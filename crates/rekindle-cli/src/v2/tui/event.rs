//! Event types for the TUI event loop.
//!
//! [`Event`] is consumed by the App main loop. Events arrive from:
//! 1. Terminal (crossterm) — keyboard, mouse, resize, paste, focus
//! 2. Internal timers — tick (state updates) and render (frame draws)
//!
//! Subscription events from the daemon arrive via a separate channel
//! and do not flow through this Event enum.

/// Unified event type for the TUI terminal event loop.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Event {
    /// Initialization complete — sent once after the event loop starts.
    Init,
    /// State update tick — fires at `tick_rate` Hz (default 4).
    Tick,
    /// Render tick — fires at `frame_rate` Hz (default 30).
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
