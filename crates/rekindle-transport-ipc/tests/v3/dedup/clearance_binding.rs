use rekindle_transport_ipc::v3::dedup::clearance_binding::check_clearance;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

#[test]
fn same_clearance_allowed() {
    assert!(check_clearance(Clearance::Internal, Clearance::Internal));
}

#[test]
fn higher_clearance_allowed() {
    assert!(check_clearance(Clearance::Internal, Clearance::Confidential));
}

#[test]
fn lower_clearance_denied() {
    assert!(!check_clearance(Clearance::Confidential, Clearance::Internal));
}

#[test]
fn unclassified_sender_denied_for_internal_entry() {
    assert!(!check_clearance(Clearance::Internal, Clearance::Unclassified));
}

#[test]
fn top_secret_entry_requires_top_secret() {
    assert!(!check_clearance(Clearance::TopSecret, Clearance::Secret));
}

#[test]
fn clearance_check_is_gte_not_eq() {
    assert!(check_clearance(Clearance::Public, Clearance::TopSecret));
}

#[test]
fn downgraded_peer_cannot_reference_prior_entry() {
    // Entry stored at Confidential. Peer was Confidential, now downgraded to Public.
    // Peer's current clearance (Public) < entry clearance (Confidential) → denied.
    assert!(!check_clearance(Clearance::Confidential, Clearance::Public));
}

#[test]
fn cross_session_clearance_uses_entry_clearance() {
    // Entry clearance is Secret. Session clearance is Confidential.
    // Sender's current clearance (Confidential) < entry clearance (Secret) → denied.
    assert!(!check_clearance(Clearance::Secret, Clearance::Confidential));
}

#[test]
fn every_clearance_allows_itself() {
    for &c in &Clearance::ALL {
        assert!(check_clearance(c, c), "{c:?} must allow itself");
    }
}

#[test]
fn clearance_ladder_is_monotonic() {
    // Each tier allows everything at or above itself
    let tiers = Clearance::ALL;
    for (i, &entry_clearance) in tiers.iter().enumerate() {
        for (j, &sender_clearance) in tiers.iter().enumerate() {
            let result = check_clearance(entry_clearance, sender_clearance);
            if j >= i {
                assert!(result, "sender {sender_clearance:?} must be allowed for entry {entry_clearance:?}");
            } else {
                assert!(!result, "sender {sender_clearance:?} must be denied for entry {entry_clearance:?}");
            }
        }
    }
}
