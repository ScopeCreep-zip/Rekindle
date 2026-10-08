//! The presence row's size budget (plan C7.15).

use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::limits::{
    MAX_BADGES, MAX_BADGE_LEN, MAX_BIO_LEN, MAX_CONTENT_REF_LEN, MAX_DISPLAY_NAME_LEN,
    MAX_PRONOUNS_LEN,
};
use rekindle_types::presence::{
    EncryptedSessionExtras, HistoryRange, MemberPresence, MemberSession, SessionExtras,
    SessionLocation,
};

use super::{fit_history_ad, resolve_superseded, signed_len, RowTarget, PRESENCE_ROW_CAP};
use crate::community::test_fixture::{MockCommunityDeps, MockState};
use crate::deps::RowWrite;

/// A route blob larger than any seen (733 B general, 976 B media in the
/// two-machine run): one crypto kind, first-hop node info included.
const ROUTE_BUDGET: usize = 1100;

/// Every field the writer sets, at its bound, in four-byte characters.
fn worst_case_row() -> MemberPresence {
    let wide = |n: usize| "🔥".repeat(n);
    let extras = SessionExtras {
        location: Some(SessionLocation::Voice {
            channel_id: "c".repeat(32),
        }),
        activity: None,
    };
    let extras_len = serde_json::to_vec(&extras).unwrap().len();
    MemberPresence {
        pseudonym_key: PseudonymKey([0xAB; 32]),
        display_name: Some(wide(MAX_DISPLAY_NAME_LEN)),
        status: "online".into(),
        route_blob: vec![0xFF; ROUTE_BUDGET],
        last_heartbeat: u64::MAX,
        avatar_ref: Some("a".repeat(MAX_CONTENT_REF_LEN)),
        banner_ref: Some("b".repeat(MAX_CONTENT_REF_LEN)),
        bio: Some(wide(MAX_BIO_LEN)),
        pronouns: Some(wide(MAX_PRONOUNS_LEN)),
        theme_color: Some(u32::MAX),
        badges: (0..MAX_BADGES).map(|_| "x".repeat(MAX_BADGE_LEN)).collect(),
        voice_channel_id: Some("c".repeat(32)),
        session: MemberSession {
            last_active: u64::MAX,
            ..MemberSession::default()
        },
        session_extras_encrypted: Some(EncryptedSessionExtras {
            mek_generation: u64::MAX,
            ciphertext: vec![0; 12 + extras_len + 16],
        }),
        ..Default::default()
    }
}

#[test]
fn worst_case_row_fits_its_slot() {
    let len = signed_len(&worst_case_row());
    assert!(
        len <= PRESENCE_ROW_CAP,
        "worst-case presence row is {len} B, the slot holds {PRESENCE_ROW_CAP} B"
    );
}

#[test]
fn the_cap_is_the_registry_subkey_limit() {
    assert_eq!(PRESENCE_ROW_CAP, 4112);
}

fn range(channel: u8, newest: u64) -> HistoryRange {
    HistoryRange {
        channel_id: ChannelId([channel; 16]),
        oldest_lamport: 1,
        newest_lamport: newest,
    }
}

/// At the bound of every field the slot has no room left for even one
/// range (3,990 of 4,112 B): the ad is the part that yields.
#[test]
fn a_worst_case_row_carries_no_history_ad() {
    let deps = MockCommunityDeps::new(MockState::default());
    let ranges: Vec<HistoryRange> = (0..3u8).map(|c| range(c, u64::from(c))).collect();
    assert!(fit_history_ad(&deps, "c1", &worst_case_row(), ranges).is_none());
}

/// A typical row (ASCII name and bio, the measured 733 B route) has room
/// for some ranges, not 40, and keeps the most recently active.
#[test]
fn history_ad_keeps_the_newest_ranges_that_fit() {
    let deps = MockCommunityDeps::new(MockState::default());
    let row = MemberPresence {
        display_name: Some("FireStarter92".into()),
        bio: Some("x".repeat(120)),
        route_blob: vec![1; 733],
        ..Default::default()
    };
    let ranges: Vec<HistoryRange> = (0..40u8).map(|c| range(c, u64::from(c))).collect();

    let ad = fit_history_ad(&deps, "c1", &row, ranges).expect("some ranges fit");
    let mut with_ad = row.clone();
    with_ad.history_ranges_encrypted = Some(ad.clone());
    assert!(signed_len(&with_ad) <= PRESENCE_ROW_CAP);

    // Each attempt encrypts the newest `count` ranges, and the one before
    // the attempt that overflowed is what was kept.
    let st = deps.state.lock();
    let tried: Vec<usize> = st.calls_encrypt_history.iter().map(|(_, n)| *n).collect();
    let kept = tried[tried.len() - 2];
    assert!(kept > 0 && kept < 40, "kept {kept} of 40");
    let newest_kept: Vec<HistoryRange> = (0..40u8)
        .rev()
        .take(kept)
        .map(|c| range(c, u64::from(c)))
        .collect();
    assert_eq!(
        ad.ciphertext.len(),
        12 + serde_json::to_vec(&newest_kept).unwrap().len() + 16
    );
}

#[test]
fn a_small_row_carries_every_range() {
    let deps = MockCommunityDeps::new(MockState::default());
    let row = MemberPresence {
        route_blob: vec![1; 733],
        signature: Vec::new(),
        ..Default::default()
    };
    let ranges: Vec<HistoryRange> = (0..5u8).map(|c| range(c, u64::from(c))).collect();
    let ad = fit_history_ad(&deps, "c1", &row, ranges.clone()).expect("fits");
    let all = 12 + serde_json::to_vec(&ranges).unwrap().len() + 16;
    assert_eq!(ad.ciphertext.len(), all);
}

/// A validly signed row by the pseudonym derived from `secret`, and that
/// pseudonym's hex.
fn signed_row(secret: u8) -> (Vec<u8>, String) {
    let key = rekindle_secrets::derive::derive_community_pseudonym(&[secret; 32], "c1");
    let mut row = MemberPresence {
        pseudonym_key: PseudonymKey(key.verifying_key().to_bytes()),
        route_blob: vec![1; 16],
        ..Default::default()
    };
    row.signature =
        rekindle_secrets::derive::sign_with_pseudonym(&key, &row.signing_bytes()).to_vec();
    (
        serde_json::to_vec(&row).unwrap(),
        hex::encode(row.pseudonym_key.0),
    )
}

fn target(my_pseudonym_hex: &str) -> RowTarget<'_> {
    RowTarget {
        community_id: "c1",
        registry_key: "reg",
        subkey: 0,
        writer: "kp",
        my_pseudonym_hex,
    }
}

fn writes(deps: &MockCommunityDeps) -> usize {
    deps.state.lock().calls_write_registry.len()
}

/// Our own newer copy held the slot: write again at once.
#[tokio::test]
async fn superseded_by_our_own_row_is_rewritten() {
    let deps = MockCommunityDeps::new(MockState::default());
    let (ours, me) = signed_row(1);
    resolve_superseded(&deps, &target(&me), b"{}".to_vec(), Some(184), &ours).await;
    assert_eq!(writes(&deps), 1, "one rewrite");
}

/// Another member's validly signed row: a slot collision, never fought.
#[tokio::test]
async fn superseded_by_another_member_is_not_overwritten() {
    let deps = MockCommunityDeps::new(MockState::default());
    let (theirs, _) = signed_row(2);
    let (_, me) = signed_row(1);
    resolve_superseded(&deps, &target(&me), b"{}".to_vec(), Some(9), &theirs).await;
    assert_eq!(writes(&deps), 0);
}

/// A value that does not verify is not overwritten either.
#[tokio::test]
async fn superseded_by_an_unverified_value_is_not_overwritten() {
    let deps = MockCommunityDeps::new(MockState::default());
    let (ours, me) = signed_row(1);
    // Signed content changed after signing.
    let forged = String::from_utf8(ours.clone())
        .unwrap()
        .replacen("\"status\":\"online\"", "\"status\":\"away\"", 1)
        .into_bytes();
    assert_ne!(forged, ours);
    resolve_superseded(&deps, &target(&me), b"{}".to_vec(), Some(9), &forged).await;
    resolve_superseded(&deps, &target(&me), b"{}".to_vec(), Some(9), b"not json").await;
    assert_eq!(writes(&deps), 0);
}

/// A rewrite that is itself superseded stops there; the next tick writes.
#[tokio::test]
async fn a_rewrite_is_attempted_once() {
    let (ours, me) = signed_row(1);
    let mut state = MockState::default();
    state.write_outcomes.push_back(RowWrite::Superseded {
        seq: Some(185),
        data: ours.clone(),
    });
    let deps = MockCommunityDeps::new(state);
    resolve_superseded(&deps, &target(&me), b"{}".to_vec(), Some(184), &ours).await;
    assert_eq!(writes(&deps), 1);
}
