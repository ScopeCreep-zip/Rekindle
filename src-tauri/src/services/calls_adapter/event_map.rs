//! The `CallSignalEvent` → desktop event mapping behind
//! `CallSignalingDeps::emit_event` (split out of [`super::deps_impl`] by
//! responsibility): each signaling event becomes the `CallEvent` (and, for
//! an incoming ring, the `NotificationEvent`) the call windows consume,
//! delivered per window through `event_dispatch` (ADR 0013).

use rekindle_calls::signaling::CallSignalEvent;
use rekindle_calls::state::CallKind;
use rekindle_types::subscription_events::{CallEvent, CallOutcome};

use super::CallsAdapter;

impl CallsAdapter {
    /// Translate one signaling event into the desktop's typed events.
    pub(super) fn emit_signal_event(&self, event: CallSignalEvent) {
        let kind_str = |k: CallKind| match k {
            CallKind::Audio => "audio".to_string(),
            CallKind::Video => "video".to_string(),
        };
        let kind_u8_str = |k: u8| match k {
            0 => "audio".to_string(),
            _ => "video".to_string(),
        };

        match event {
            CallSignalEvent::IncomingCall {
                call_id,
                from_public_key,
                from_display_name,
                kind,
                expires_at_ms,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Incoming {
                        call_id: call_id.clone(),
                        from: from_public_key.clone(),
                        display_name: from_display_name.clone(),
                        kind: kind_str(kind),
                        // A 1:1 call lists nobody: the caller is
                        // already `from`, and the callee is us.
                        participants: Vec::new(),
                        is_group: false,
                        expires_at_ms,
                        group_id: None,
                    },
                );
                crate::event_dispatch::emit_notification(
                    &self.app_handle,
                    rekindle_types::subscription_events::NotificationEvent::CallIncoming {
                        call_id,
                        display_name: from_display_name,
                        from: from_public_key,
                        kind: kind_str(kind),
                        expires_at_ms,
                        is_group: false,
                    },
                );
            }
            CallSignalEvent::CallRinging { call_id, .. } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Ringing { call_id },
                );
            }
            CallSignalEvent::CallConnected {
                call_id,
                peer_public_key,
                peer_display_name,
                kind,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Connected {
                        call_id,
                        group_id: None,
                        direct: Some(rekindle_types::subscription_events::DirectCallInfo {
                            kind: kind_str(kind),
                            peer_key: peer_public_key,
                            peer_display_name,
                            expected_local_camera: matches!(kind, CallKind::Video),
                        }),
                    },
                );
            }
            CallSignalEvent::CallDeclined {
                call_id,
                peer_reason,
                ..
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    CallEvent::Declined {
                        call_id,
                        notice: CallOutcome::Declined.notice(),
                        peer_reason: (!peer_reason.is_empty()).then_some(peer_reason),
                    },
                );
            }
            // One arm for both: a call ending is the same fact
            // whether it had two participants or ten, and the desktop's
            // two variants had identical fields.
            CallSignalEvent::CallEnded {
                call_id,
                peer_reason,
                ..
            }
            | CallSignalEvent::GroupCallEnded {
                call_id,
                peer_reason,
            } => {
                crate::windows::retire_call_window(&self.app_handle, &call_id);
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    CallEvent::Ended {
                        call_id,
                        notice: CallOutcome::Ended.notice(),
                        peer_reason,
                    },
                );
            }
            CallSignalEvent::CallTimedOut { call_id, .. } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    CallEvent::TimedOut {
                        call_id,
                        notice: CallOutcome::TimedOut.notice(),
                    },
                );
            }
            CallSignalEvent::CallMissed {
                call_id,
                peer_public_key,
                ..
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Missed {
                        call_id,
                        from: peer_public_key,
                    },
                );
            }
            CallSignalEvent::ConversationFocusRequested {
                peer_public_key,
                peer_display_name,
                reason,
            } => {
                // Not a call event: focusing a conversation is a
                // channel-scoped UI hint, and it fires for accepted
                // friend requests too, not only for calls.
                crate::event_dispatch::emit_subscription(
                    &self.app_handle,
                    &rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                        rekindle_types::subscription_events::ChannelMessageEvent::ConversationFocusRequested {
                            peer_key: peer_public_key,
                            display_name: peer_display_name,
                            reason,
                        },
                    ),
                );
            }
            CallSignalEvent::CallStarted {
                call_id,
                peer_public_key,
                peer_display_name,
                kind,
                expires_at_ms,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Started {
                        call_id,
                        kind: kind_str(kind),
                        peer_key: peer_public_key,
                        peer_display_name,
                        expires_at_ms,
                    },
                );
            }
            CallSignalEvent::GroupCallStarted {
                call_id,
                participants,
                kind,
                expires_at_ms,
                group_id,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::GroupStarted {
                        call_id,
                        kind: kind_u8_str(kind),
                        participants,
                        group_id,
                        expires_at_ms,
                    },
                );
            }
            CallSignalEvent::IncomingGroupCall {
                call_id,
                initiator_public_key,
                initiator_display_name,
                participants,
                kind,
                expires_at_ms,
                group_id,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::Incoming {
                        call_id: call_id.clone(),
                        from: initiator_public_key.clone(),
                        display_name: initiator_display_name.clone(),
                        kind: kind_u8_str(kind),
                        participants,
                        is_group: true,
                        expires_at_ms,
                        group_id,
                    },
                );
                crate::event_dispatch::emit_notification(
                    &self.app_handle,
                    rekindle_types::subscription_events::NotificationEvent::CallIncoming {
                        call_id,
                        display_name: initiator_display_name,
                        from: initiator_public_key,
                        kind: kind_u8_str(kind),
                        expires_at_ms,
                        is_group: true,
                    },
                );
            }
            CallSignalEvent::GroupCallConnected { call_id, group_id } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    // No peer: a group call has no single other end,
                    // and the signal carries only the id.
                    rekindle_types::subscription_events::CallEvent::Connected {
                        call_id,
                        direct: None,
                        group_id,
                    },
                );
            }
            CallSignalEvent::GroupCallParticipantJoined {
                call_id,
                peer_public_key,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::ParticipantJoined {
                        call_id,
                        participant_pubkey: peer_public_key,
                    },
                );
            }
            CallSignalEvent::GroupCallParticipantLeft {
                call_id,
                peer_public_key,
                reason,
            } => {
                crate::event_dispatch::emit_call(
                    &self.app_handle,
                    rekindle_types::subscription_events::CallEvent::ParticipantLeft {
                        call_id,
                        participant_pubkey: peer_public_key,
                        reason,
                    },
                );
            }
        }
    }
}
