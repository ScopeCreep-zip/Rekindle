use rekindle_types::subscription_events::{JoinHandshake, VoiceEvent, VoiceMediaReady, VoiceScope};

use super::{net, our_session_key, CommunityVoiceEvent, ControlPayload, Peer, CHANNEL, COMMUNITY};

#[tokio::test]
async fn local_joined_carries_the_handshake_from_a_join_received_during_loop_start() {
    let net = net();
    let local = Peer::new(&net, 1, false);
    let remote = Peer::new(&net, 2, false);
    local
        .shown_names
        .lock()
        .insert(remote.id(), "Remote".into());
    let session = local.transport.lock().await.session_state();

    // The receive loop processes a peer's VoiceJoin before startup can
    // publish LocalJoined. Use the real dispatcher and await the apply task
    // so the test controls the ordering without a sleep between the events.
    let deps = local.deps();
    let sender = remote.id();
    let join = ControlPayload::VoiceJoin {
        channel_id: CHANNEL.into(),
        route_blob: remote.route.clone(),
        display_name: Some("Remote".into()),
        session_key: our_session_key(&*remote, COMMUNITY, CHANNEL),
    };
    tokio::spawn(async move {
        crate::signaling::handle_voice_signaling(deps, COMMUNITY, &sender, join).await;
    })
    .await
    .expect("receive loop");
    local.settle().await;

    let stages: Vec<_> = local
        .events
        .lock()
        .iter()
        .filter_map(|event| match event {
            CommunityVoiceEvent::VoiceJoinHandshake { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(stages, [JoinHandshake::Seen, JoinHandshake::Connected]);

    let scope = VoiceScope::Community {
        community: COMMUNITY.into(),
        channel: CHANNEL.into(),
    };
    let mut joined = None;
    session.emit_local_joined(scope.clone(), |event| joined = Some(event));
    let VoiceEvent::LocalJoined {
        scope: joined_scope,
        join_handshake,
        media_ready,
    } = joined.expect("LocalJoined emitted")
    else {
        panic!("expected LocalJoined");
    };
    assert_eq!(joined_scope, scope);
    assert_eq!(join_handshake, JoinHandshake::Connected);
    assert_eq!(
        media_ready,
        VoiceMediaReady {
            ready: false,
            reason: "roster-empty".into()
        }
    );
}
