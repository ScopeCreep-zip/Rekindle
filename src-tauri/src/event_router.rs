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
/// on, the event payload, and the journal sequence number for journaled
/// events (used to skip duplicates after a reload replay).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundEnvelope {
    pub seq: Option<u64>,
    pub channel: &'static str,
    pub payload: serde_json::Value,
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
        replay: impl IntoIterator<Item = OutboundEnvelope>,
    ) {
        let mut subs = self.subs.write();
        for envelope in replay {
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

    /// Send `envelope` to every registered window in `audience`.
    pub fn deliver(&self, envelope: &OutboundEnvelope, audience: &Audience) {
        for (label, channel) in self.subs.read().iter() {
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
mod tests {
    use super::*;

    fn channel() -> Channel<OutboundEnvelope> {
        Channel::new(|_| Ok(()))
    }

    #[test]
    fn latest_register_wins_and_destroy_removes() {
        let router = WebviewRouter::default();
        router.register("chat-1".into(), channel(), []);
        router.register("chat-1".into(), channel(), []);
        router.register("buddy-list".into(), channel(), []);
        assert_eq!(
            router.labels(),
            vec!["buddy-list".to_owned(), "chat-1".to_owned()]
        );
        router.remove("chat-1");
        assert_eq!(router.labels(), vec!["buddy-list".to_owned()]);
    }

    #[test]
    fn audience_matches_labels_and_kinds() {
        let audience = Audience::labels(["chat-0123456789abcdef".to_owned()])
            .with_kinds(&[WindowKind::Community]);
        assert!(audience.includes("chat-0123456789abcdef"));
        assert!(!audience.includes("chat-fedcba9876543210"));
        assert!(audience.includes("community-browser"));
        assert!(!audience.includes("login"));
        assert!(Audience::post_auth().includes("settings"));
        assert!(!Audience::post_auth().includes("login"));
    }
}
