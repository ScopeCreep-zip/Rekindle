//! Plan GI4: the relayed ingest core runs on the state alone and returns
//! what the shell shows and what still needs the Tauri handle. With a real
//! offline Host on the state, a pin the Host accepts is exactly one social
//! event; a thread reply, voice signaling, a link preview and a
//! `RequestMEK` are exactly one follow-up each and no event; a
//! `MEKRotated` or `OnboardingComplete` frame is a hint that shows nothing
//! (the window's events come from the Host's own key install and
//! validation, plan GI3b).

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{
    CommunityEnvelope, ControlPayload, MediaSessionKey, SignedEnvelope,
};
use rekindle_node::host::test_fixtures::{create_ann, seat_creator, seed_registry_community};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

use super::{IngestOutcome, ShellFollowUp, WebviewEvent};
use crate::test_support::TestHost;

const SEATED_MEMBER: PseudonymKey = PseudonymKey([3; 32]);

/// A Host holding Ann's community `c1`, with `SEATED_MEMBER` seated as a creator.
async fn host_with_member() -> TestHost {
    let test = TestHost::start().await;
    let owner = create_ann(&test.host).await.public_key;
    seed_registry_community(&test.host, &owner).await;
    seat_creator(&test.host, "c1", &SEATED_MEMBER);
    test
}

/// `payload` relayed by `SEATED_MEMBER` in `c1`, through the relayed core.
async fn ingest(test: &TestHost, payload: ControlPayload) -> IngestOutcome {
    let signed = SignedEnvelope {
        community_id: "c1".into(),
        sender_pseudonym: SEATED_MEMBER.to_hex(),
        message_id: [1; rekindle_codec::envelope::MESSAGE_ID_LEN],
        envelope_bytes: encode_community_envelope(&CommunityEnvelope::control(payload))
            .expect("encodes"),
        signature: Vec::new(),
        ttl: 0,
    };
    super::relayed_core(&test.state, signed).await
}

#[tokio::test]
async fn a_pin_the_host_accepts_is_exactly_one_social_event() {
    let test = host_with_member().await;
    let out = ingest(
        &test,
        ControlPayload::MessagePinned {
            channel_id: "general".into(),
            message_id: "m1".into(),
            pinned_by: SEATED_MEMBER.to_hex(),
        },
    )
    .await;
    assert!(out.shell.is_empty(), "a pin needs no shell follow-up");
    assert_eq!(out.events.len(), 1, "one event for one accepted pin");
    assert!(
        matches!(
            &out.events[0],
            WebviewEvent::Subscription(SubscriptionEvent::Social(SocialEvent::MessagePinned {
                community, message_id, ..
            })) if community == "c1" && message_id == "m1"
        ),
        "the event is the accepted pin"
    );
    test.shutdown().await;
}

#[tokio::test]
async fn a_pin_not_the_senders_shows_nothing() {
    let test = host_with_member().await;
    let out = ingest(
        &test,
        ControlPayload::MessagePinned {
            channel_id: "general".into(),
            message_id: "m1".into(),
            pinned_by: hex::encode([9u8; 32]),
        },
    )
    .await;
    assert!(out.events.is_empty() && out.shell.is_empty());
    test.shutdown().await;
}

#[tokio::test]
async fn shell_arms_are_one_follow_up_and_no_event() {
    let test = host_with_member().await;
    let cases = [
        (
            ControlPayload::ThreadMessageReceived {
                thread_id: "t1".into(),
                message_id: "m1".into(),
                sender_pseudonym: SEATED_MEMBER.to_hex(),
                timestamp: 1,
                reply_to_id: None,
            },
            "thread",
        ),
        (
            ControlPayload::VoiceJoin {
                channel_id: "voice".into(),
                route_blob: vec![1],
                display_name: None,
                session_key: MediaSessionKey::default(),
            },
            "voice",
        ),
        (
            ControlPayload::LinkPreview {
                channel_id: "general".into(),
                message_id: "m1".into(),
                url: "https://example.org".into(),
                title: None,
                description: None,
                site_name: None,
                fetched_at: 1,
            },
            "preview",
        ),
        (
            ControlPayload::RequestMEK {
                channel_id: "general".into(),
                needed_generation: 1,
                requester_pseudonym: SEATED_MEMBER.to_hex(),
                cascade_index: 0,
            },
            "mek",
        ),
    ];
    for (payload, arm) in cases {
        let out = ingest(&test, payload).await;
        assert!(out.events.is_empty(), "{arm}: no event from the core");
        assert_eq!(out.shell.len(), 1, "{arm}: exactly one follow-up");
        let matched = match (&out.shell[0], arm) {
            (ShellFollowUp::ThreadControl { community_id, .. }, "thread")
            | (ShellFollowUp::MekRequest { community_id, .. }, "mek") => community_id == "c1",
            (
                ShellFollowUp::VoiceSignaling {
                    community_id,
                    sender_pseudonym,
                    ..
                },
                "voice",
            ) => community_id == "c1" && *sender_pseudonym == SEATED_MEMBER.to_hex(),
            (
                ShellFollowUp::LinkPreview {
                    channel_id,
                    preview,
                    ..
                },
                "preview",
            ) => channel_id == "general" && preview.message_id == "m1",
            _ => false,
        };
        assert!(matched, "{arm}: the follow-up is the arm's own");
    }
    test.shutdown().await;
}

#[tokio::test]
async fn mek_rotated_and_onboarding_complete_show_nothing() {
    let test = host_with_member().await;
    for payload in [
        ControlPayload::MEKRotated {
            channel_id: None,
            new_generation: 2,
            rotator_pseudonym: Some(SEATED_MEMBER.to_hex()),
        },
        ControlPayload::OnboardingComplete {
            pseudonym_key: SEATED_MEMBER.to_hex(),
            role_ids: Vec::new(),
        },
    ] {
        let out = ingest(&test, payload).await;
        assert!(
            out.events.is_empty() && out.shell.is_empty(),
            "a hint frame shows nothing"
        );
    }
    test.shutdown().await;
}
