use super::*;
use rekindle_types::presence::SessionLocation;

fn full_session() -> MemberSession {
    MemberSession {
        status: SessionStatus::Online,
        location: Some(SessionLocation::Text {
            channel_id: "chan".to_string(),
        }),
        activity: Some("Playing Halo".to_string()),
        last_active: 1_700_003_661, // not on an hour boundary
    }
}

#[test]
fn default_policy_redacts_location_activity_and_lastseen_but_stays_online() {
    let policy = PresenceSharingPolicy::default();
    let out = apply_sharing_policy(full_session(), &policy);
    assert_eq!(out.status, SessionStatus::Online, "online is the baseline");
    assert!(out.location.is_none(), "default-deny strips location");
    assert!(out.activity.is_none(), "default-deny strips activity");
    assert_eq!(out.last_active, 0, "default hides last-seen");
}

#[test]
fn share_online_off_folds_to_invisible() {
    let policy = PresenceSharingPolicy {
        share_online: false,
        ..PresenceSharingPolicy::default()
    };
    let out = apply_sharing_policy(full_session(), &policy);
    assert_eq!(out.status, SessionStatus::Invisible);
    assert_eq!(out.status.as_wire_str(), "offline");
}

#[test]
fn opted_in_signals_pass_through() {
    let policy = PresenceSharingPolicy {
        share_online: true,
        share_location: ShareScope::Members,
        share_activity: ShareScope::Members,
        last_seen: LastSeenPrecision::Exact,
    };
    let session = full_session();
    let original_active = session.last_active;
    let out = apply_sharing_policy(session, &policy);
    assert!(out.location.is_some());
    assert_eq!(out.activity.as_deref(), Some("Playing Halo"));
    assert_eq!(out.last_active, original_active, "Exact is verbatim");
}

#[test]
fn coarse_lastseen_is_bucketed_not_zero() {
    let policy = PresenceSharingPolicy {
        share_online: true,
        last_seen: LastSeenPrecision::Coarse,
        ..PresenceSharingPolicy::default()
    };
    let out = apply_sharing_policy(full_session(), &policy);
    assert_ne!(out.last_active, 0);
    assert_eq!(out.last_active % HOUR_SECS, 0, "bucketed to the hour");
}

#[test]
fn filter_incoming_drops_non_reciprocated_peer_signals() {
    // We share nothing extra (default). A peer who shares everything
    // still shows us nothing beyond status.
    let our_policy = PresenceSharingPolicy::default();
    let peer = full_session();
    let filtered = filter_incoming(peer, &our_policy);
    assert!(filtered.location.is_none());
    assert!(filtered.activity.is_none());
    assert_eq!(filtered.last_active, 0);
}

#[test]
fn filter_incoming_keeps_reciprocated_signals() {
    let our_policy = PresenceSharingPolicy {
        share_online: true,
        share_location: ShareScope::Members,
        share_activity: ShareScope::Members,
        last_seen: LastSeenPrecision::Coarse,
    };
    let peer = full_session();
    let filtered = filter_incoming(peer, &our_policy);
    assert!(filtered.location.is_some());
    assert!(filtered.activity.is_some());
    assert_ne!(filtered.last_active, 0);
}

#[test]
fn coarsen_never_emits_sub_hour_precision() {
    for secs in [0u64, 1, 59, 3599, 3600, 3601, 1_700_003_661] {
        let bucket = coarsen_to_bucket(secs);
        assert_eq!(bucket % HOUR_SECS, 0);
        assert!(bucket <= secs, "bucket floors, never rounds up");
        assert!(secs - bucket < HOUR_SECS);
    }
}
