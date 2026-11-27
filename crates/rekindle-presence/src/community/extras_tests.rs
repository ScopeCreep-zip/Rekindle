//! Our beat's sealed extras (plan E3.4 S8-pre): the sharing policy first,
//! sealed under the community key, unreadable after a rotation, inside the
//! fixed 512 B beat.

use rekindle_codec::member_record::beat::{MAX_ACTIVITY, MAX_CHANNEL_ID};
use rekindle_codec::member_record::{BeatState, PresenceBeat, RecordScope, PRESENCE_BEAT_BYTES};
use rekindle_db::test_support::Seeded;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::presence::{
    LastSeenPrecision, MemberSession, PresenceSharingPolicy, SessionExtras, SessionLocation,
    ShareScope,
};

use super::{open_extras, seal_extras};
use crate::deps::{SessionPresence, VoicePresenceRow};

/// The member record our sealed extras are bound to.
const EXTRAS_SCOPE: RecordScope<'static> = RecordScope {
    community_id: "community",
    member_record: "VLD0:member-record",
};

/// Our session: the longest location and activity, under `policy`.
struct Session {
    policy: PresenceSharingPolicy,
}

impl SessionPresence for Session {
    fn active_voice_channel(&self, _community_id: &str) -> Option<String> {
        Some("v".repeat(MAX_CHANNEL_ID))
    }

    fn reconcile_voice_roster(&self, _community_id: &str, _rows: Vec<VoicePresenceRow>) {}

    fn self_session(&self, _community_id: &str) -> MemberSession {
        MemberSession {
            location: Some(SessionLocation::Text {
                channel_id: "c".repeat(MAX_CHANNEL_ID),
            }),
            activity: Some("a".repeat(MAX_ACTIVITY)),
            ..MemberSession::default()
        }
    }

    fn presence_policy(&self, _community_id: &str) -> PresenceSharingPolicy {
        self.policy.clone()
    }
}

fn sharing_all() -> PresenceSharingPolicy {
    PresenceSharingPolicy {
        share_online: true,
        share_location: ShareScope::Members,
        share_activity: ShareScope::Members,
        last_seen: LastSeenPrecision::Exact,
    }
}

/// The default policy shares no location and no activity: the beat
/// carries no extras at all, even with the key held.
#[test]
fn the_default_policy_seals_no_extras() {
    let session = Session {
        policy: PresenceSharingPolicy::default(),
    };
    let key = MediaEncryptionKey::from_bytes([3; 32], 1);
    assert_eq!(
        seal_extras(
            &session,
            EXTRAS_SCOPE.community_id,
            Some(&key),
            EXTRAS_SCOPE,
            &Seeded::new(1)
        ),
        None
    );
}

/// A shared activity only: the location the policy withholds is not in
/// the sealed extras.
#[test]
fn the_extras_carry_only_what_the_policy_shares() {
    let session = Session {
        policy: PresenceSharingPolicy {
            share_location: ShareScope::None,
            ..sharing_all()
        },
    };
    let key = MediaEncryptionKey::from_bytes([3; 32], 1);
    let sealed = seal_extras(
        &session,
        EXTRAS_SCOPE.community_id,
        Some(&key),
        EXTRAS_SCOPE,
        &Seeded::new(1),
    )
    .unwrap();
    assert_eq!(sealed.mek_generation, 1);
    assert_eq!(
        open_extras(&sealed, Some(&key), EXTRAS_SCOPE, &sharing_all()),
        Some(SessionExtras {
            location: None,
            activity: Some("a".repeat(MAX_ACTIVITY)),
        })
    );
    // Without a key nothing is sealed: extras are never written in the clear.
    assert_eq!(
        seal_extras(
            &session,
            EXTRAS_SCOPE.community_id,
            None,
            EXTRAS_SCOPE,
            &Seeded::new(1)
        ),
        None
    );
}

/// After a rotation the reader holds only the new generation: the extras
/// sealed under the old one do not open, and a reader whose own policy
/// shares nothing keeps nothing.
#[test]
fn the_extras_do_not_open_after_a_rotation() {
    let session = Session {
        policy: sharing_all(),
    };
    let old = MediaEncryptionKey::from_bytes([3; 32], 1);
    let sealed = seal_extras(
        &session,
        EXTRAS_SCOPE.community_id,
        Some(&old),
        EXTRAS_SCOPE,
        &Seeded::new(1),
    )
    .unwrap();
    assert!(open_extras(&sealed, Some(&old), EXTRAS_SCOPE, &sharing_all()).is_some());
    let rotated = MediaEncryptionKey::from_bytes([4; 32], 2);
    assert_eq!(
        open_extras(&sealed, Some(&rotated), EXTRAS_SCOPE, &sharing_all()),
        None
    );
    assert_eq!(
        open_extras(&sealed, None, EXTRAS_SCOPE, &sharing_all()),
        None
    );
    assert_eq!(
        open_extras(
            &sealed,
            Some(&old),
            EXTRAS_SCOPE,
            &PresenceSharingPolicy::default()
        ),
        None
    );
}

/// The beat with the longest voice channel id and the fullest extras our
/// session can seal is still exactly 512 B.
#[test]
fn the_beat_stays_512_bytes_with_the_longest_voice_channel_and_full_extras() {
    let session = Session {
        policy: sharing_all(),
    };
    let key = MediaEncryptionKey::from_bytes([3; 32], u64::MAX);
    let extras = seal_extras(
        &session,
        EXTRAS_SCOPE.community_id,
        Some(&key),
        EXTRAS_SCOPE,
        &Seeded::new(1),
    );
    assert!(extras.is_some());
    let beat = PresenceBeat {
        seq: u64::MAX,
        state: BeatState::Busy,
        voice_channel: session.active_voice_channel(EXTRAS_SCOPE.community_id),
        beat_number: u64::MAX,
        extras,
    };
    let bytes = beat
        .encode(EXTRAS_SCOPE, &SigningKey::from_bytes(&[1; 32]))
        .unwrap();
    assert_eq!(bytes.len(), PRESENCE_BEAT_BYTES);
}
