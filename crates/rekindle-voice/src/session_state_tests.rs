use super::*;

#[test]
fn local_join_and_both_transition_families_emit_under_the_session_lock() {
    let session = VoiceSessionState::default();
    let mut stages = Vec::new();
    assert!(session.advance_handshake(JoinHandshake::Seen, |media| {
        assert!(session.state.try_lock().is_none());
        stages.push(media.expect("readiness changed").reason);
    }));

    let scope = VoiceScope::Community {
        community: "c".into(),
        channel: "ch".into(),
    };
    session.emit_local_joined(scope.clone(), |event| {
        assert!(session.state.try_lock().is_none());
        let VoiceEvent::LocalJoined {
            scope: joined_scope,
            join_handshake,
            media_ready,
        } = event
        else {
            panic!("expected LocalJoined");
        };
        assert_eq!(joined_scope, scope);
        assert_eq!(join_handshake, JoinHandshake::Seen);
        assert_eq!(
            media_ready,
            VoiceMediaReady {
                ready: false,
                reason: "handshake-seen".into()
            }
        );
    });

    assert!(
        session.advance_handshake(JoinHandshake::Connected, |media| {
            stages.push(media.expect("readiness changed").reason);
        })
    );
    session.update_media_ready(
        |inputs| inputs.roster_non_empty = true,
        |status| {
            assert!(session.state.try_lock().is_none());
            stages.push(status.reason);
        },
    );
    session.update_media_ready(
        |inputs| inputs.local_caps_reported = true,
        |status| {
            stages.push(status.reason);
        },
    );
    session.update_media_ready(
        |inputs| inputs.session_config_emitted = true,
        |status| {
            assert!(status.ready);
            stages.push(status.reason);
        },
    );
    assert_eq!(session.media_ready_gate(), Ok(()));
    session.update_media_ready(
        |inputs| inputs.session_config_emitted = true,
        |_| {
            panic!("unchanged readiness was emitted");
        },
    );
    session.update_media_ready(
        |inputs| inputs.roster_non_empty = false,
        |status| {
            assert!(!status.ready);
            stages.push(status.reason);
        },
    );
    assert_eq!(
        stages,
        [
            "handshake-seen",
            "roster-empty",
            "caps-unreported",
            "config-pending",
            "ready",
            "roster-empty",
        ]
    );
    session.end(|_| panic!("already not ready"));
    assert_eq!(session.media_ready_gate(), Err("not-in-voice".into()));
}

#[test]
fn ending_a_ready_session_emits_once_and_disarms_late_signaling() {
    let session = VoiceSessionState::default();
    session.advance_handshake(JoinHandshake::Connected, |_| {});
    session.update_media_ready(
        |inputs| {
            inputs.roster_non_empty = true;
            inputs.local_caps_reported = true;
            inputs.session_config_emitted = true;
        },
        |_| {},
    );
    session.end(|status| {
        assert!(session.state.try_lock().is_none());
        assert_eq!(
            status,
            VoiceMediaReady {
                ready: false,
                reason: "left-channel".into()
            }
        );
    });
    session.end(|_| panic!("session already ended"));
    session.update_media_ready(|_| panic!("session already ended"), |_| {});
    assert_eq!(session.media_ready_gate(), Err("not-in-voice".into()));
}
