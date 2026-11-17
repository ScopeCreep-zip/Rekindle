//! Cross-platform alert state management for `WebDriver`.
//!
//! This module provides per-window state for handling JavaScript alert/confirm/prompt dialogs
//! across different platform implementations.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

/// Type of pending alert
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertType {
    Alert,
    Confirm,
    Prompt,
}

/// Response from `WebDriver` for an alert
#[derive(Debug, Clone)]
pub struct AlertResponse {
    pub accepted: bool,
    pub prompt_text: Option<String>,
}

/// Pending alert waiting for `WebDriver` response
pub struct PendingAlert {
    pub message: String,
    pub default_text: Option<String>,
    pub alert_type: AlertType,
    /// Sender to signal `WebDriver`'s accept/dismiss response
    pub responder: std::sync::mpsc::Sender<AlertResponse>,
}

/// Per-window alert state for coordinating between UI delegate and `WebDriver` commands
pub struct AlertState {
    pending: Mutex<Option<PendingAlert>>,
    /// Text input for prompt dialogs (set by `sendAlertText`)
    prompt_input: Mutex<Option<String>>,
}

impl AlertState {
    /// Create a new empty alert state
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
            prompt_input: Mutex::new(None),
        }
    }

    /// Set a pending alert, clearing any previous prompt input
    pub fn set_pending(&self, alert: PendingAlert) {
        *self.prompt_input.lock() = None;
        *self.pending.lock() = Some(alert);
    }

    /// Set the prompt input text (only valid for Prompt type alerts)
    pub fn set_prompt_input(&self, text: String) -> bool {
        // Only allow setting if there's a pending prompt
        if !matches!(self.get_alert_type(), Some(AlertType::Prompt)) {
            return false;
        }
        *self.prompt_input.lock() = Some(text);
        true
    }

    /// Get the current prompt input text
    pub fn get_prompt_input(&self) -> Option<String> {
        self.prompt_input.lock().clone()
    }

    /// Get the message of the current pending alert
    pub fn get_message(&self) -> Option<String> {
        self.pending.lock().as_ref().map(|a| a.message.clone())
    }

    /// Get the type of the current pending alert
    pub fn get_alert_type(&self) -> Option<AlertType> {
        self.pending.lock().as_ref().map(|a| a.alert_type)
    }

    /// Get the default text for prompt dialogs
    pub fn get_default_text(&self) -> Option<String> {
        self.pending
            .lock()
            .as_ref()
            .and_then(|a| a.default_text.clone())
    }

    /// Send response to pending alert and clear it
    pub fn respond(&self, accepted: bool, prompt_text: Option<String>) -> bool {
        let Some(alert) = self.pending.lock().take() else {
            return false;
        };
        *self.prompt_input.lock() = None;
        let _ = alert.responder.send(AlertResponse {
            accepted,
            prompt_text,
        });
        true
    }
}

impl Default for AlertState {
    fn default() -> Self {
        Self::new()
    }
}

/// Manager for per-window alert states
pub struct AlertStateManager {
    states: Mutex<HashMap<String, Arc<AlertState>>>,
}

impl AlertStateManager {
    /// Create a new alert state manager
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create alert state for a window
    pub fn get_or_create(&self, window_label: &str) -> Arc<AlertState> {
        let mut states = self.states.lock();
        states
            .entry(window_label.to_string())
            .or_insert_with(|| Arc::new(AlertState::new()))
            .clone()
    }
}

impl Default for AlertStateManager {
    fn default() -> Self {
        Self::new()
    }
}
