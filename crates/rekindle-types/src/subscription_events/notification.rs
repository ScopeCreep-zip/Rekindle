//! Device-level notifications — the things a frontend surfaces to the
//! *user* rather than applies to its state.
//!
//! ## Why this family exists
//!
//! [`super::SystemEvent`] already covered protocol facts scoped to a
//! community (announcements, raid alerts, bootstrap and sync receipts).
//! It had nothing for "tell the person something": an incoming call, a
//! message worth a notification sound, an app update, a session-reset
//! request needing a human decision.
//!
//! The desktop carried those in a Tauri-only `channels::NotificationEvent`,
//! so a CLI could not ring, alert, or prompt without reimplementing the
//! protocol logic behind each one. That was never the intent — the
//! desktop's own `CallIncoming` comment said this variant existed *"so a
//! CLI / TUI frontend can hook its own notifier (terminal bell,
//! libnotify, etc.) without re-implementing protocol logic"*, which is
//! precisely what a Tauri-only enum prevents.
//!
//! ## Wire constraints
//!
//! Same as [`super::presence`]: postcard on the daemon IPC, so no
//! `#[serde(flatten)]` and no `tag = "..."` enums.

use serde::{Deserialize, Serialize};

/// Something to surface to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NotificationEvent {
    /// A message arrived that is worth notifying about.
    MessageReceived {
        title: String,
        body: String,
        community_id: String,
        channel_id: String,
        /// Resolved via channel override → community default → `None`
        /// (the frontend falls back to its bundled default).
        sound_ref: Option<String>,
    },

    /// An app-level alert that belongs to no channel — network
    /// connectivity, a decrypt failure, and the like.
    SystemAlert { title: String, body: String },

    /// A newer release is available.
    UpdateAvailable { version: String },

    /// A peer sent a `SessionResetRequest`.
    ///
    /// Needs a human decision: the user must compare `safety_number`
    /// with the peer out-of-band before accepting, so a frontend has to
    /// prompt rather than auto-accept.
    SessionResetRequested {
        peer_public_key: String,
        peer_display_name: String,
        /// Hex-encoded short safety number (8 hex chars = 32 bits) for
        /// out-of-band comparison — bigger than a phone number, small
        /// enough to read aloud. BLAKE3 of the sorted identity key pair,
        /// so both sides compute the same value.
        safety_number: String,
    },

    /// An incoming call, on the ring channel.
    ///
    /// Distinct from the in-app modal that
    /// [`super::ChannelMessageEvent`] drives: this is the notifier
    /// layer, so a terminal bell and a desktop banner hang off the same
    /// event. `call_id` lets a frontend correlate ringtone start and
    /// stop to the call's lifecycle; `is_group` distinguishes a 1:1
    /// modal from a group banner.
    CallIncoming {
        call_id: String,
        from: String,
        display_name: String,
        kind: String,
        expires_at_ms: u64,
        is_group: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postcard_round_trips_every_variant() {
        let events = vec![
            NotificationEvent::MessageReceived {
                title: "t".into(),
                body: "b".into(),
                community_id: "c".into(),
                channel_id: "ch".into(),
                sound_ref: Some("ping".into()),
            },
            NotificationEvent::SystemAlert {
                title: "t".into(),
                body: "b".into(),
            },
            NotificationEvent::UpdateAvailable {
                version: "1.2.3".into(),
            },
            NotificationEvent::SessionResetRequested {
                peer_public_key: "pk".into(),
                peer_display_name: "Ada".into(),
                safety_number: "deadbeef".into(),
            },
            NotificationEvent::CallIncoming {
                call_id: "id".into(),
                from: "pk".into(),
                display_name: "Ada".into(),
                kind: "audio".into(),
                expires_at_ms: 1234,
                is_group: false,
            },
        ];
        for event in events {
            let bytes = postcard::to_allocvec(&event).expect("postcard encode");
            let back: NotificationEvent = postcard::from_bytes(&bytes).expect("postcard decode");
            assert_eq!(format!("{event:?}"), format!("{back:?}"));
        }
    }

    /// Pins the JSON `src/ipc/channels/notification_events.ts` parses.
    #[test]
    fn json_shape_is_what_the_webview_parses() {
        let event = NotificationEvent::UpdateAvailable {
            version: "1.2.3".into(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"updateAvailable":{"version":"1.2.3"}}"#
        );

        let call = NotificationEvent::CallIncoming {
            call_id: "id".into(),
            from: "pk".into(),
            display_name: "Ada".into(),
            kind: "audio".into(),
            expires_at_ms: 1234,
            is_group: true,
        };
        assert_eq!(
            serde_json::to_string(&call).unwrap(),
            r#"{"callIncoming":{"callId":"id","from":"pk","displayName":"Ada","kind":"audio","expiresAtMs":1234,"isGroup":true}}"#
        );
    }
}
