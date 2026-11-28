//! A missed join is repaired by presence, then completed by the peer's ack.

use super::{net, CommunityVoiceEvent, ControlPayload, Peer, CHANNEL, COMMUNITY};
use crate::signaling::{handle_voice_signaling, reconcile_from_presence, PresencePeerView};

#[tokio::test]
async fn reconcile_join_uses_the_host_card_name() {
    repaired_join("Cara").await;
}

#[tokio::test]
async fn reconcile_join_uses_the_host_fingerprint_without_a_card_name() {
    repaired_join("ed49 28c6").await;
}

async fn repaired_join(shown_name: &str) {
    let net = net();
    let ari = Peer::new(&net, 1, false);
    let cara = Peer::new(&net, 3, false);
    let cara_key = cara.id();
    ari.shown_names
        .lock()
        .insert(cara_key.clone(), shown_name.into());

    reconcile_from_presence(
        &ari.deps(),
        COMMUNITY,
        vec![PresencePeerView {
            pseudonym_hex: cara_key.clone(),
            voice_channel_id: Some(CHANNEL.into()),
            fresh: true,
        }],
    )
    .await;
    let announcement = ari
        .mesh
        .lock()
        .pop()
        .expect("reconcile reannounced our join");
    assert!(matches!(
        announcement.as_control(),
        Some(ControlPayload::VoiceJoin { .. })
    ));

    // Cara replies to that join. Her unsigned name differs from the Host view.
    handle_voice_signaling(
        ari.deps(),
        COMMUNITY,
        &cara_key,
        ControlPayload::VoiceJoinAck {
            channel_id: CHANNEL.into(),
            joiner_pseudonym: ari.id(),
            display_name: Some("Handshake name".into()),
            route_blob: cara.route.clone(),
            session_key: super::our_session_key(&*cara, COMMUNITY, CHANNEL),
        },
    )
    .await;
    ari.settle().await;

    let events = ari.events.lock();
    let joined_name = events.iter().find_map(|event| match event {
        CommunityVoiceEvent::VoiceJoin {
            pseudonym_key,
            display_name,
            ..
        } if pseudonym_key == &cara_key => Some(display_name.as_str()),
        _ => None,
    });
    assert_eq!(joined_name, Some(shown_name));
    let (changed_name, roster) = events
        .iter()
        .find_map(|event| match event {
            CommunityVoiceEvent::VoiceRosterChanged {
                pseudonym_key,
                present: true,
                display_name,
                roster,
                ..
            } if pseudonym_key == &cara_key => Some((display_name, roster)),
            _ => None,
        })
        .expect("the repaired join added Cara to the roster");
    assert_eq!(changed_name, shown_name);
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].pseudonym_key, cara_key);
    assert_eq!(roster[0].display_name, shown_name);
}
