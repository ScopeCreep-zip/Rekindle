//! Event types for the TUI terminal event loop.
//!
//! [`Event`] carries user input from the terminal to the machine loop.
//! Only real user actions flow through this channel — keyboard, mouse,
//! resize, paste, focus. Timer events (tick, render, status poll) are
//! owned by the machine loop's own tokio::time::Interval arms.
//!
//! This separation prevents select loop starvation: timer events at 34Hz
//! on the unbounded channel would starve daemon_rx if they shared the
//! same select arm as user input.

/// Terminal input event — user actions only, no timers.
#[derive(Clone, Debug)]
pub enum Event {
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
