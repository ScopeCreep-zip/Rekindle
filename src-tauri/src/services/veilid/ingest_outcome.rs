//! What one ingested datagram produced (plan GI4): the webview events the
//! shell shows and the follow-ups that still need the Tauri handle. The
//! ingest cores (`app_message::ingest_item` and the control cores under it)
//! take only the state and return an [`IngestOutcome`]; the worker's
//! `process_ingress_item` hands it to [`IngestOutcome::deliver`], which emits the
//! events through `event_dispatch::emit_from_state` and runs each follow-up
//! through the adapter that owns it, so the cores are tested without an
//! `AppHandle`.

use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_db::Db;
use rekindle_transport::operations::direct::Opened;
use rekindle_types::link_preview::LinkPreview;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};
use tauri::AppHandle;

use crate::event_dispatch::WebviewEvent;
use crate::state::AppState;

/// Work an ingest core hands to an adapter that keeps an `AppHandle`.
pub(crate) enum ShellFollowUp {
    /// A 1:1 kind the Host handed back unported (`inbound_unported`).
    Unported { pool: Db, opened: Box<Opened> },
    /// A member's `RequestMEK`, answered by the Host over the `MekAdapter`.
    MekRequest {
        community_id: String,
        payload: ControlPayload,
    },
    /// A thread reply from its authenticated sender, stored by the Host's
    /// channel adapter.
    ThreadControl {
        community_id: String,
        sender_pseudonym: String,
        payload: ControlPayload,
    },
    /// Voice signaling, run by the voice signaling adapter.
    VoiceSignaling {
        community_id: String,
        sender_pseudonym: String,
        payload: ControlPayload,
    },
    /// Channel video media or control, run by the video adapter.
    VideoPayload {
        community_id: String,
        sender_pseudonym: String,
        payload: ControlPayload,
    },
    /// A peer's link preview, checked and stored by `link_previews`.
    LinkPreview {
        community_id: String,
        sender_pseudonym: String,
        channel_id: String,
        preview: LinkPreview,
    },
}

/// The events and follow-ups one ingested datagram produced.
#[derive(Default)]
pub(crate) struct IngestOutcome {
    pub events: Vec<WebviewEvent>,
    pub shell: Vec<ShellFollowUp>,
}

impl IngestOutcome {
    /// One event, nothing else.
    pub(crate) fn event(event: WebviewEvent) -> Self {
        Self {
            events: vec![event],
            shell: Vec::new(),
        }
    }

    /// The Host's accepted social change, if it accepted one.
    pub(crate) fn social(event: Option<SocialEvent>) -> Self {
        event.map_or_else(Self::default, |event| {
            Self::event(WebviewEvent::Subscription(SubscriptionEvent::Social(event)))
        })
    }

    /// One follow-up, nothing else.
    pub(crate) fn follow_up(follow_up: ShellFollowUp) -> Self {
        Self {
            events: Vec::new(),
            shell: vec![follow_up],
        }
    }

    /// Emit the events, then run each follow-up.
    pub(crate) async fn deliver(self, app: &AppHandle, state: &Arc<AppState>) {
        for event in self.events {
            crate::event_dispatch::emit_from_state(state, event);
        }
        for follow_up in self.shell {
            follow_up.run(app, state).await;
        }
    }
}

impl ShellFollowUp {
    async fn run(self, app: &AppHandle, state: &Arc<AppState>) {
        match self {
            Self::Unported { pool, opened } => {
                super::inbound_unported::handle(app, state, &pool, *opened).await;
            }
            Self::MekRequest {
                community_id,
                payload,
            } => {
                super::control::handle_mek_request(app, state, &community_id, payload).await;
            }
            Self::ThreadControl {
                community_id,
                sender_pseudonym,
                payload,
            } => {
                super::control_events::handle_thread_payload(
                    app,
                    state,
                    &community_id,
                    &sender_pseudonym,
                    payload,
                );
            }
            Self::VoiceSignaling {
                community_id,
                sender_pseudonym,
                payload,
            } => {
                if let Some(host) = state.host.current() {
                    host.ingest_voice_signaling(&community_id, &sender_pseudonym, payload);
                }
            }
            Self::VideoPayload {
                community_id,
                sender_pseudonym,
                payload,
            } => {
                crate::services::community::video::handle_video_payload(
                    app,
                    state,
                    &community_id,
                    &sender_pseudonym,
                    payload,
                );
            }
            Self::LinkPreview {
                community_id,
                sender_pseudonym,
                channel_id,
                preview,
            } => {
                crate::services::community::link_previews::handle_incoming_link_preview(
                    app,
                    state,
                    &community_id,
                    &sender_pseudonym,
                    channel_id,
                    preview,
                );
            }
        }
    }
}
