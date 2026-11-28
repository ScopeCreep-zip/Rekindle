//! `ControlPayload::into_event` — messages, MEK management, roles, reactions and
//! pins. The remaining categories (events, threads, game servers,
//! voice, bootstrap, sync, system) are
//! handled by `into_event_control_rest.rs`.

use rekindle_types::subscription_events::{
    CryptoEvent, MembershipEvent, SocialEvent, SubscriptionEvent,
};

use rekindle_codec::community::envelope::ControlPayload;

use super::into_event_control_rest::control_into_event_rest;

/// Convert a control payload into a subscription event.
///
/// Exhaustive over the canonical `ControlPayload` — every variant has a
/// match arm, and a new one is a compile error here.
///
/// `None` when the payload is not a subscription event: nineteen
/// variants are media- or transport-plane traffic — voice signaling,
/// video fragments and FEC, attachment chunks, bandwidth and topology
/// reports — consumed by the subsystems that own them and never
/// surfaced to a subscriber — the three thread payloads, whose
/// events the Host publishes once it has stored them, and the three
/// membership notices, whose verdicts the Host publishes from the merged
/// governance. Saying so in the return type is what stops
/// them being given an invented event just to satisfy the match.
pub fn control_into_event(
    payload: ControlPayload,
    community: &str,
    sender: &str,
) -> Option<SubscriptionEvent> {
    let c = || community.to_string();
    let s = || sender.to_string();

    Some(match payload {
        // ── MEK management ──────────────────────────────────
        ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            ..
        } => SubscriptionEvent::Crypto(CryptoEvent::MekRequested {
            community: c(),
            channel: channel_id,
            needed_generation,
            requester_pseudonym,
        }),
        // Protocol's is a tuple variant wrapping the whole payload;
        // transport's near-copy flattened it. The payload carries the
        // channel as `Option<String>` because the community-wide key has
        // none, so an absent channel means "the community MEK".
        ControlPayload::MekTransfer(transfer) => {
            SubscriptionEvent::Crypto(CryptoEvent::MekTransferred {
                community: transfer.community_id,
                channel: transfer.channel_id,
                generation: transfer.generation,
                sender_pseudonym: transfer.sender_pseudonym,
            })
        }

        // ── Onboarding ──────────────────────────────────────
        ControlPayload::SubmitOnboardingAnswers { answers } => {
            SubscriptionEvent::Membership(MembershipEvent::OnboardingAnswersSubmitted {
                community: c(),
                sender_pseudonym: s(),
                answer_count: answers.len(),
            })
        }

        // ── Reactions & pins ────────────────────────────────
        ControlPayload::ReactionAdded {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => SubscriptionEvent::Social(SocialEvent::ReactionAdded {
            community: c(),
            channel: channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        }),
        ControlPayload::ReactionRemoved {
            channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        } => SubscriptionEvent::Social(SocialEvent::ReactionRemoved {
            community: c(),
            channel: channel_id,
            message_id,
            emoji,
            reactor_pseudonym,
        }),
        ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        } => SubscriptionEvent::Social(SocialEvent::MessagePinned {
            community: c(),
            channel: channel_id,
            message_id,
            pinned_by,
        }),
        ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        } => SubscriptionEvent::Social(SocialEvent::MessageUnpinned {
            community: c(),
            channel: channel_id,
            message_id,
        }),

        // The remaining categories (events, game servers, voice,
        // bootstrap, sync, system)
        // are mapped by `into_event_rest` to keep this method under the
        // clippy::too_many_lines limit. They are listed explicitly rather
        // than via `_` so that adding a `ControlPayload` variant remains a
        // compile error here — exhaustiveness is preserved.
        ControlPayload::EventCreated { .. }
        | ControlPayload::EventUpdated { .. }
        | ControlPayload::EventDeleted { .. }
        | ControlPayload::EventRsvpChanged { .. }
        | ControlPayload::GameServerAdded { .. }
        | ControlPayload::GameServerRemoved { .. }
        | ControlPayload::VoiceLeave { .. }
        | ControlPayload::VoiceModeSwitch { .. }
        | ControlPayload::VoiceMute { .. }
        | ControlPayload::VoiceDeafen { .. }
        | ControlPayload::BootstrapRequest { .. }
        | ControlPayload::BootstrapResponseV2 { .. }
        // Stage signalling. These are gossiped decisions every member
        // observes, not local session state — the desktop surfaced them
        // and the daemon dropped them, so a TUI could not render a
        // stage at all.
        | ControlPayload::StageUpdate { .. }
        | ControlPayload::SpeakRequest { .. }
        | ControlPayload::SpeakResponse { .. }
        | ControlPayload::SoundboardPlay { .. } => {
            control_into_event_rest(payload, community, sender)
        }

        // ── Not subscription events ──────────────────────────
        //
        // Media and transport plane. Each is consumed by the subsystem
        // that owns it — `rekindle-voice` signaling, the video
        // fragment/FEC pipeline, the files chunk transfer, the MEK
        // rotation ack path — and none has a meaning in a subscriber's
        // event stream. Listed by name rather than caught by `_` so a
        // new variant still has to be classified here.
        //
        // Threads (plan E3.1 E3f): the Host's channel adapter stores each
        // one and publishes its event. A reply's notification carries the
        // sealed body but not the subkey and Lamport position its AAD
        // binds, so only the Host, reading the reply from the thread
        // record, has the body the event carries; mapping them here as
        // well would publish each twice, the reply once without its body.
        //
        // Membership verdicts (A/B F7b-1, F7b-2): who joined, left, was
        // kicked, banned or unbanned is the merged governance's to say,
        // and the Host publishes each from the merge, once, with the
        // remover and the reason (`host/adapters/governance/member_events.rs`).
        // A gossip notice is a hint that is not authenticated for policy
        // and carries neither; mapping it too published every removal
        // twice, first without its reason.
        ControlPayload::MemberJoined { .. }
        | ControlPayload::MemberLeave { .. }
        | ControlPayload::Kick { .. }
        // A key's arrival and an onboarding verdict are the Host's to say
        // too: `MekRotated` when it installs the key (`mek_transfer.rs`) or
        // mints it (`mek_rotation`), `OnboardingCompleted` from its own
        // check of the answers (`membership_events/onboarding.rs`). The
        // notice is unauthenticated as to either and is a hint only.
        | ControlPayload::MEKRotated { .. }
        | ControlPayload::OnboardingComplete { .. }
        | ControlPayload::ThreadMessageReceived { .. }
        | ControlPayload::MekTransferAck(_)
        | ControlPayload::RequestSegmentExpansion { .. }
        // Named voice events are built by the Host from verified cards.
        | ControlPayload::VoiceJoin { .. }
        | ControlPayload::VoiceRoster { .. }
        | ControlPayload::VoiceJoinAck { .. }
        | ControlPayload::VoiceJoinConfirmed { .. }
        | ControlPayload::VoiceMediaKey { .. }
        | ControlPayload::VoiceMediaKeyRequest { .. }
        | ControlPayload::VoiceMediaKeyAck { .. }
        // A governance op reaches the Host's op store, whose rebuild emits
        // what changed (plan E3.2).
        | ControlPayload::GovernanceOp { .. }
        | ControlPayload::HeadAdvanced { .. }
        // A snapshot request and its answer travel by app_call (S7c.3).
        | ControlPayload::CheckpointStateRequest { .. }
        | ControlPayload::CheckpointStateResponse { .. }
        | ControlPayload::CheckpointHintRequest { .. }
        | ControlPayload::CheckpointHintResponse { .. }
        // A frontier request and its answer travel by app_call (S7d-keys.8).
        | ControlPayload::FrontierRequest { .. }
        | ControlPayload::FrontierResponse { .. }
        // A partial responder's bootstrap answer, read by the joiner's call.
        | ControlPayload::BootstrapPartial { .. }
        | ControlPayload::BootstrapRefused { .. }
        | ControlPayload::RequestAttachment { .. }
        | ControlPayload::AttachmentChunk { .. }
        | ControlPayload::MultiAttachmentChunk { .. }
        | ControlPayload::VideoFragment(_)
        | ControlPayload::VideoParityFragment(_)
        | ControlPayload::KeyframeRequest { .. }
        | ControlPayload::MediaCapabilities { .. }
        | ControlPayload::TopologyChange { .. }
        | ControlPayload::LinkPreview { .. } => return None,
    })
}
