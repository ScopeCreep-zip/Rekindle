use rekindle_codec::community::envelope::{ControlPayload, MediaSessionKey, VoiceRosterEntry};
use rekindle_governance_runtime::CardText;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent, VoiceParticipant};

use crate::host::runtime::tests::offline_host;

#[tokio::test]
async fn voice_join_prefers_the_verified_card_to_the_handshake() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let cara = PseudonymKey([0xab; 32]);
    assert!(host.ctx.communities.note_member_card(
        "community",
        &cara,
        CardText {
            display_name: Some("Cara".into()),
            custom_status: None,
            game: None
        },
    ));
    let event = host.governance().named_voice_event(
        "community",
        &cara.to_hex(),
        &ControlPayload::VoiceJoin {
            channel_id: "stage".into(),
            route_blob: vec![1, 2, 3],
            display_name: Some("Handshake name".into()),
            session_key: MediaSessionKey::default(),
        },
    );
    let Some(SubscriptionEvent::Voice(VoiceEvent::Joined {
        display_name,
        route_blob,
        ..
    })) = event
    else {
        panic!("expected a voice join");
    };
    assert_eq!(display_name, "Cara");
    assert_eq!(route_blob, Some(vec![1, 2, 3]));
}

#[tokio::test]
async fn voice_roster_uses_the_fingerprint_when_the_card_has_no_name() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let cara = PseudonymKey([0xab; 32]);
    assert!(host.ctx.communities.note_member_card(
        "community",
        &cara,
        CardText {
            display_name: None,
            custom_status: None,
            game: None
        },
    ));
    let event = host.governance().named_voice_event(
        "community",
        &cara.to_hex(),
        &ControlPayload::VoiceRoster {
            channel_id: "stage".into(),
            participants: [cara.to_hex(), "invalid-key".into()]
                .into_iter()
                .map(|pseudonym_key| VoiceRosterEntry {
                    pseudonym_key,
                    route_blob: vec![1, 2, 3],
                    muted: false,
                    deafened: false,
                    display_name: Some("Handshake name".into()),
                    session_key: MediaSessionKey::default(),
                })
                .collect(),
        },
    );
    let Some(SubscriptionEvent::Voice(VoiceEvent::RosterUpdated { participants, .. })) = event
    else {
        panic!("expected a roster update");
    };
    assert_eq!(
        participants,
        vec![VoiceParticipant {
            pseudonym_key: cara.to_hex(),
            display_name: "abab abab".into(),
        }]
    );
}
