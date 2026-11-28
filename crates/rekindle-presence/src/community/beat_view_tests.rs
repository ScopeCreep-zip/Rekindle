use rekindle_codec::member_record::{BeatState, PresenceBeat, RecordScope, BEAT_PERIOD_MS};

use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::presence::{MemberSession, PresenceSharingPolicy, SessionLocation, ShareScope};
use rekindle_utils::random::OsRandom;

use crate::community::{open_extras, seal_extras};

use super::{BeatView, Observed, BEAT_LIVENESS_MS};

const MEMBER: &str = "aa";
/// A period start, on our clock.
const PERIOD_START: u64 = 5_871_000 * BEAT_PERIOD_MS;

fn beat(seq: u64, state: BeatState, at_ms: u64) -> PresenceBeat {
    PresenceBeat {
        seq,
        state,
        voice_channel: None,
        beat_number: at_ms / BEAT_PERIOD_MS,
        extras: None,
    }
}

/// A member whose beats stop turns offline 15 minutes after its last beat
/// arrived, with no new write: not before, and exactly once.
#[test]
fn a_member_whose_beats_stop_turns_offline_locally() {
    let mut view = BeatView::default();
    let seen = view.observe(
        MEMBER,
        &beat(1, BeatState::Online, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    assert_eq!(
        seen,
        Observed {
            state_changed: true,
            shown: Some(BeatState::Online),
            voice_changed: false,
        }
    );
    assert_eq!(view.next_expiry(), Some(PERIOD_START + BEAT_LIVENESS_MS));
    assert!(view.expire(PERIOD_START + BEAT_LIVENESS_MS - 1).is_empty());
    assert_eq!(view.shown(MEMBER), BeatState::Online);
    assert_eq!(
        view.expire(PERIOD_START + BEAT_LIVENESS_MS),
        vec![MEMBER.to_owned()]
    );
    assert_eq!(view.shown(MEMBER), BeatState::Offline);
    assert!(view.expire(PERIOD_START + 2 * BEAT_LIVENESS_MS).is_empty());
    assert_eq!(view.next_expiry(), None);
}

/// A beat number an hour ahead of our clock counts from its arrival (Q4 as
/// amended): its member is offline 15 minutes after it arrived, not an
/// hour and a quarter later.
#[test]
fn a_beat_number_an_hour_ahead_expires_from_its_arrival() {
    let mut view = BeatView::default();
    let ahead = beat(1, BeatState::Online, PERIOD_START + 60 * 60 * 1000);
    let seen = view.observe(MEMBER, &ahead, Some(PERIOD_START), PERIOD_START);
    assert_eq!(seen.shown, Some(BeatState::Online));
    assert_eq!(view.next_expiry(), Some(PERIOD_START + BEAT_LIVENESS_MS));
    assert_eq!(
        view.expire(PERIOD_START + BEAT_LIVENESS_MS),
        vec![MEMBER.to_owned()]
    );
}

/// A newer beat moves the expiry; an older or repeated one changes nothing.
#[test]
fn only_a_newer_beat_counts() {
    let mut view = BeatView::default();
    view.observe(
        MEMBER,
        &beat(5, BeatState::Online, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    let later = PERIOD_START + BEAT_PERIOD_MS;
    let refresh = view.observe(
        MEMBER,
        &beat(6, BeatState::Online, later),
        Some(later),
        later,
    );
    assert_eq!(refresh, Observed::default(), "a refresh is no transition");
    assert_eq!(view.next_expiry(), Some(later + BEAT_LIVENESS_MS));
    let stale = view.observe(
        MEMBER,
        &beat(4, BeatState::Busy, PERIOD_START),
        Some(later),
        later,
    );
    assert_eq!(stale, Observed::default());
    assert_eq!(view.shown(MEMBER), BeatState::Online);
}

/// An `Offline` (quit) or `Departed` (leave) beat shows offline at once;
/// a member never seen shows offline and its first `Offline` beat is no
/// change of what it shows.
#[test]
fn an_offline_or_departed_beat_shows_offline_at_once() {
    let mut view = BeatView::default();
    let first = view.observe(
        MEMBER,
        &beat(1, BeatState::Offline, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    assert_eq!(first.shown, None);
    assert!(first.state_changed);
    view.observe(
        MEMBER,
        &beat(2, BeatState::Away, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    let quit = view.observe(
        MEMBER,
        &beat(3, BeatState::Departed, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    assert_eq!(quit.shown, Some(BeatState::Offline));
    assert_eq!(view.next_expiry(), None);
}

/// A cold read (no arrival) counts from the beat number alone: an old beat
/// read for the first time shows its member offline.
#[test]
fn a_cold_read_counts_from_the_beat_number() {
    let mut view = BeatView::default();
    let old = beat(1, BeatState::Online, PERIOD_START - BEAT_LIVENESS_MS);
    assert_eq!(view.observe(MEMBER, &old, None, PERIOD_START).shown, None);
    assert_eq!(view.shown(MEMBER), BeatState::Offline);
    let fresh = beat(2, BeatState::Online, PERIOD_START);
    assert_eq!(
        view.observe(MEMBER, &fresh, None, PERIOD_START).shown,
        Some(BeatState::Online)
    );
}

/// A member none of whose beats reached us may be present (unknown is not
/// offline, plan E3.4 S5 mesh fix 1); one whose beat expired, or whose
/// beat says Offline or Departed, may not; a live Online beat may.
#[test]
fn may_be_present_keeps_the_unknown_and_drops_the_gone() {
    let mut view = BeatView::default();
    assert!(view.may_be_present(MEMBER), "never seen");
    view.observe(
        MEMBER,
        &beat(1, BeatState::Online, PERIOD_START),
        Some(PERIOD_START),
        PERIOD_START,
    );
    assert!(view.may_be_present(MEMBER), "a live beat");
    view.expire(PERIOD_START + BEAT_LIVENESS_MS);
    assert!(!view.may_be_present(MEMBER), "an expired beat");
    for (seq, state) in [(2, BeatState::Offline), (3, BeatState::Departed)] {
        let mut view = BeatView::default();
        view.observe(
            MEMBER,
            &beat(seq, state, PERIOD_START),
            Some(PERIOD_START),
            PERIOD_START,
        );
        assert!(!view.may_be_present(MEMBER), "{state:?}");
    }
}

fn voice_beat(seq: u64, state: BeatState, at_ms: u64, voice: Option<&str>) -> PresenceBeat {
    PresenceBeat {
        voice_channel: voice.map(str::to_owned),
        ..beat(seq, state, at_ms)
    }
}

fn claims(view: &BeatView) -> Vec<(String, Option<String>, bool)> {
    (view.voice_rows().into_iter())
        .map(|r| (r.pseudonym_hex, r.voice_channel_id, r.fresh))
        .collect()
}

/// A member's beat naming a voice channel puts it in that channel for the
/// voice roster (plan E3.4 S8-pre); its expiry takes it out with no new
/// write, so a ghost is expired.
#[test]
fn a_voice_beat_claims_its_channel_until_it_expires() {
    let mut view = BeatView::default();
    let seen = view.observe(
        MEMBER,
        &voice_beat(1, BeatState::Online, PERIOD_START, Some("voice")),
        Some(PERIOD_START),
        PERIOD_START,
    );
    assert!(seen.voice_changed);
    assert_eq!(
        claims(&view),
        vec![(MEMBER.to_owned(), Some("voice".to_owned()), true)]
    );
    assert_eq!(view.expire(PERIOD_START + BEAT_LIVENESS_MS), vec![MEMBER]);
    assert_eq!(claims(&view), vec![(MEMBER.to_owned(), None, false)]);
}

/// A leave beat (no channel) takes the member out of the channel at once;
/// a beat that repeats the same channel changes no claim.
#[test]
fn a_leave_beat_ends_the_voice_claim() {
    let mut view = BeatView::default();
    view.observe(
        MEMBER,
        &voice_beat(1, BeatState::Online, PERIOD_START, Some("voice")),
        Some(PERIOD_START),
        PERIOD_START,
    );
    let same = view.observe(
        MEMBER,
        &voice_beat(2, BeatState::Online, PERIOD_START, Some("voice")),
        Some(PERIOD_START),
        PERIOD_START + 1,
    );
    assert!(!same.voice_changed);
    let left = view.observe(
        MEMBER,
        &voice_beat(3, BeatState::Online, PERIOD_START, None),
        Some(PERIOD_START),
        PERIOD_START + 2,
    );
    assert!(left.voice_changed);
    assert_eq!(claims(&view), vec![(MEMBER.to_owned(), None, true)]);
    // A quit's offline beat that still names the channel claims nothing.
    view.observe(
        MEMBER,
        &voice_beat(4, BeatState::Offline, PERIOD_START, Some("voice")),
        Some(PERIOD_START),
        PERIOD_START + 3,
    );
    assert_eq!(claims(&view), vec![(MEMBER.to_owned(), None, false)]);
}

#[test]
fn opens_selected_devices_extras_at_its_own_scope_under_reciprocity() {
    let primary = RecordScope {
        community_id: "community",
        member_record: "VLD0:primary",
    };
    let secondary = RecordScope {
        community_id: "community",
        member_record: "VLD0:replacement",
    };
    let key = MediaEncryptionKey::from_bytes([42; 32], 6);
    let rotated = MediaEncryptionKey::from_bytes([43; 32], 7);
    let policy = PresenceSharingPolicy {
        share_location: ShareScope::Members,
        ..PresenceSharingPolicy::default()
    };
    let location = SessionLocation::Text {
        channel_id: "general".into(),
    };
    let sealed = seal_extras(
        MemberSession {
            location: Some(location.clone()),
            ..MemberSession::default()
        },
        &policy,
        Some(&key),
        secondary,
        &OsRandom,
    )
    .expect("the writer shared location using the effective device record");
    let mut view = BeatView::default();
    let primary_beat = PresenceBeat {
        seq: 100,
        state: BeatState::Online,
        voice_channel: None,
        beat_number: 0,
        extras: None,
    };
    let secondary_beat = PresenceBeat {
        seq: 1,
        extras: Some(sealed),
        ..primary_beat.clone()
    };
    view.observe_record("peer", primary.member_record, &primary_beat, Some(200), 200);
    view.observe_record(
        "peer",
        secondary.member_record,
        &secondary_beat,
        Some(201),
        201,
    );
    let (record, ciphertext) = view
        .sealed_extras("peer")
        .expect("the selected live device has extras");
    let selected = RecordScope {
        community_id: primary.community_id,
        member_record: record,
    };
    assert_eq!(record, secondary.member_record);
    assert_eq!(
        open_extras(ciphertext, Some(&key), selected, &policy)
            .unwrap()
            .location,
        Some(location)
    );
    assert!(open_extras(ciphertext, Some(&key), primary, &policy).is_none());
    let other_key = MediaEncryptionKey::from_bytes([43; 32], 6);
    assert!(open_extras(ciphertext, Some(&other_key), selected, &policy).is_none());
    assert!(open_extras(ciphertext, Some(&rotated), selected, &policy).is_none());
    assert!(open_extras(
        ciphertext,
        Some(&key),
        selected,
        &PresenceSharingPolicy::default()
    )
    .is_none());
    let later = 202;
    view.observe_record(
        "peer",
        primary.member_record,
        &PresenceBeat {
            seq: 101,
            ..primary_beat
        },
        Some(later),
        later,
    );
    assert!(
        view.sealed_extras("peer").is_none(),
        "the selected record has no extras"
    );
    view.retain_records(|_, record| record != primary.member_record);
    assert_eq!(
        view.sealed_extras("peer").map(|(record, _)| record),
        Some(secondary.member_record)
    );
    assert_eq!(view.expire(201 + BEAT_LIVENESS_MS), ["peer"]);
    assert!(view.sealed_extras("peer").is_none());
}
