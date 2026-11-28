//! The daemon's voice record (plan E3.4 S8-pre): what our beat's
//! `voiceChannel` reads on `rekindled`.

use rekindle_presence::SessionPresence;
use rekindle_transport::operations::voice::VoiceSession;

use super::OwnVoice;

const A: &str = "community-a";
const B: &str = "community-b";
const JOINED: &str = "0123456789abcdef0123456789abcdef";

fn session(community: &str) -> VoiceSession {
    VoiceSession {
        community_id: community.to_owned(),
        channel_id: JOINED.to_owned(),
        our_route_blob: vec![7; 16],
        muted: false,
        deafened: false,
    }
}

#[test]
fn a_join_puts_its_channel_in_that_community_beat_only() {
    let voice = OwnVoice::default();
    assert!(voice.join(session(A)).is_none());
    assert_eq!(voice.active_voice_channel(A).as_deref(), Some(JOINED));
    assert_eq!(voice.active_voice_channel(B), None);
}

#[test]
fn a_leave_clears_the_channel_and_ends_the_session() {
    let voice = OwnVoice::default();
    voice.join(session(A));
    let left = voice.leave().expect("the session we were in");
    assert_eq!(left.community_id, A);
    assert!(left.our_route_blob.is_empty(), "leave_voice ended it");
    assert_eq!(voice.active_voice_channel(A), None);
    assert!(voice.leave().is_none(), "nothing left to leave");
}

#[test]
fn a_join_elsewhere_ends_the_session_it_replaces() {
    let voice = OwnVoice::default();
    voice.join(session(A));
    let replaced = voice.join(session(B)).expect("the session in A");
    assert_eq!(replaced.community_id, A);
    assert!(replaced.our_route_blob.is_empty());
    assert_eq!(voice.active_voice_channel(A), None);
    assert_eq!(voice.active_voice_channel(B).as_deref(), Some(JOINED));
}

#[test]
fn leaving_a_community_ends_only_a_session_in_it() {
    let voice = OwnVoice::default();
    voice.join(session(A));
    assert!(voice.leave_community(B).is_none());
    assert_eq!(voice.active_voice_channel(A).as_deref(), Some(JOINED));
    assert!(voice.leave_community(A).is_some());
    assert_eq!(voice.active_voice_channel(A), None);
}
