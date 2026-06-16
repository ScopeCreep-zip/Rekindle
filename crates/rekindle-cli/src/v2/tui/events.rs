//! Typed event enums — the state machine's input alphabet.

use rekindle_types::daemon::DaemonResponse;
use rekindle_types::subscription_events::SubscriptionEvent;

use super::state::ephemeral::ToastLevel;

pub enum TerminalEvent {
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Resize(u16, u16),
    Paste(String),
    FocusGained,
    FocusLost,
}

pub enum DaemonEvent {
    Subscription(SubscriptionEvent),
    /// Raw IPC response. Deserialized into typed result by process_command_result
    /// using the RequestKind looked up from in_flight by request_id.
    CommandResult { request_id: u64, response: DaemonResponse },
    CommandFailed { request_id: u64, error: String },
    ConnectionLost,
}

pub enum InternalEvent {
    Toast { message: String, level: ToastLevel },
}
