//! Per-window event delivery.
//!
//! Each webview opens one [`tauri::ipc::Channel`] with `subscribe_events`;
//! the router keeps it under the window's label and sends each event only
//! to the windows in its [`Audience`]. Tauri's global `listen` delivered
//! every event to every webview regardless of `emit_to` (a JS `listen`
//! registers `EventTarget::Any`), so a chat window received another
//! peer's messages and every window ran its own copy of the call UI.
//!
//! A page reload keeps the label but not the channel, and Tauri cannot
//! tell the old channel is dead, so the latest `register` for a label
//! wins. `WindowEvent::Destroyed` removes the entry.

use std::collections::HashMap;

use serde::Serialize;
use tauri::ipc::Channel;

use crate::window_labels::WindowKind;

/// What a webview receives: the frontend channel name its handlers listen
/// on, the event payload, and, for an event the host journaled, its cursor
/// (plan D3.2: the window keeps the last one, replays from it after a
/// reload, and skips an event it has already seen).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundEnvelope {
    pub cursor: Option<rekindle_types::subscription_events::Cursor>,
    pub channel: &'static str,
    pub payload: serde_json::Value,
    /// Part of a standing snapshot (`crate::event_snapshot`), not a live
    /// or replayed event.
    pub standing: bool,
}

/// The channel of the stream's own frames: `{"invalidSession": null}` (the
/// window reloads) and `{"resumed": cursor}`.
pub const STREAM_CHANNEL: &str = "event-stream";

impl OutboundEnvelope {
    /// The window's cursor cannot be served: it reloads.
    #[must_use]
    pub fn invalid_session() -> Self {
        Self {
            cursor: None,
            channel: STREAM_CHANNEL,
            payload: serde_json::json!({ "invalidSession": null }),
            standing: false,
        }
    }

    /// The replay is complete up to `cursor`.
    #[must_use]
    pub fn resumed(cursor: rekindle_types::subscription_events::Cursor) -> Self {
        Self {
            cursor: Some(cursor),
            channel: STREAM_CHANNEL,
            payload: serde_json::json!({ "resumed": cursor }),
            standing: false,
        }
    }

    /// This envelope as part of a standing snapshot.
    #[must_use]
    pub fn into_standing(self) -> Self {
        Self {
            standing: true,
            ..self
        }
    }
}

/// The windows an event is for: specific labels, plus every open window
/// of the listed kinds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Audience {
    labels: Vec<String>,
    kinds: Vec<WindowKind>,
}

impl Audience {
    #[must_use]
    pub fn labels(labels: impl IntoIterator<Item = String>) -> Self {
        Self {
            labels: labels.into_iter().collect(),
            kinds: Vec::new(),
        }
    }

    #[must_use]
    pub fn kinds(kinds: &[WindowKind]) -> Self {
        Self {
            labels: Vec::new(),
            kinds: kinds.to_vec(),
        }
    }

    /// Every window shown after login.
    #[must_use]
    pub fn post_auth() -> Self {
        Self::kinds(
            &WindowKind::ALL
                .into_iter()
                .filter(|k| *k != WindowKind::Login)
                .collect::<Vec<_>>(),
        )
    }

    #[must_use]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = String>) -> Self {
        self.labels.extend(labels);
        self
    }

    #[must_use]
    pub fn with_kinds(mut self, kinds: &[WindowKind]) -> Self {
        self.kinds.extend_from_slice(kinds);
        self
    }

    #[must_use]
    pub fn includes(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
            || WindowKind::of_label(label).is_some_and(|k| self.kinds.contains(&k))
    }
}

/// Live channels by window label.
#[derive(Default)]
pub struct WebviewRouter {
    subs: parking_lot::RwLock<HashMap<String, Channel<OutboundEnvelope>>>,
}

impl WebviewRouter {
    /// Register `channel` for `label`, replacing any earlier one, then send
    /// `replay` on it before any live event can be delivered to it.
    pub fn register(
        &self,
        label: String,
        channel: Channel<OutboundEnvelope>,
        snapshot: impl FnOnce() -> Vec<OutboundEnvelope>,
    ) {
        let mut subs = self.subs.write();
        for envelope in snapshot() {
            if let Err(e) = channel.send(envelope) {
                tracing::debug!(window = %label, error = %e, "event replay send failed");
            }
        }
        subs.insert(label, channel);
    }

    /// Forget a destroyed window.
    pub fn remove(&self, label: &str) {
        self.subs.write().remove(label);
    }

    /// Send `envelope` to every registered window.
    pub fn deliver_all(&self, envelope: &OutboundEnvelope) {
        for (label, channel) in self.subs.read().iter() {
            if let Err(e) = channel.send(envelope.clone()) {
                tracing::debug!(window = %label, error = %e, "event send failed");
            }
        }
    }

    /// Send `envelope` to every registered window in `audience`.
    pub fn deliver(&self, envelope: &OutboundEnvelope, audience: &Audience) {
        self.deliver_if(envelope, audience, || true);
    }

    /// Recheck a live event under the same dispatch lock that orders new
    /// subscribers' snapshots, then deliver it to the matching windows.
    pub fn deliver_if(
        &self,
        envelope: &OutboundEnvelope,
        audience: &Audience,
        is_current: impl FnOnce() -> bool,
    ) {
        let subs = self.subs.read();
        if !is_current() {
            return;
        }
        Self::send_matching(&subs, audience, envelope);
    }

    /// Read a live snapshot under the lock that orders subscriber snapshots.
    pub fn deliver_snapshot(
        &self,
        audience: &Audience,
        snapshot: impl FnOnce() -> Option<OutboundEnvelope>,
    ) {
        let subs = self.subs.read();
        if let Some(envelope) = snapshot() {
            Self::send_matching(&subs, audience, &envelope);
        }
    }

    fn send_matching(
        subs: &HashMap<String, Channel<OutboundEnvelope>>,
        audience: &Audience,
        envelope: &OutboundEnvelope,
    ) {
        for (label, channel) in subs {
            if audience.includes(label) {
                if let Err(e) = channel.send(envelope.clone()) {
                    tracing::debug!(window = %label, error = %e, "event send failed");
                }
            }
        }
    }

    #[cfg(test)]
    fn labels(&self) -> Vec<String> {
        let mut labels: Vec<_> = self.subs.read().keys().cloned().collect();
        labels.sort();
        labels
    }
}

#[cfg(test)]
#[path = "event_router_tests.rs"]
mod tests;
