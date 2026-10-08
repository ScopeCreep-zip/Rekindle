//! Admission merge tests: join requests, decisions, and the immutability
//! of the admission mode.

use super::*;
use rekindle_types::governance::AdmissionMode;

/// Genesis metadata plus an all-permissions `everyone` role, so the
/// creator can write decision entries. Mirrors the other test modules.
fn genesis() -> Vec<GovernanceEntry> {
    vec![
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: role_id(0),
            name: "everyone".into(),
            permissions: rekindle_types::permissions::ALL,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
    ]
}

#[test]
fn join_request_must_be_self_authored() {
    let creator = pseudo(1);
    let joiner = pseudo(2);
    let impostor = pseudo(3);

    let mut creator_entries = genesis();
    // The creator forges a request in the joiner's name.
    creator_entries.push(GovernanceEntry::JoinRequested {
        requester: joiner.clone(),
        display_name: "not-really-ada".into(),
        lamport: 3,
    });

    let state = merge(&[
        (creator, creator_entries),
        (
            impostor,
            vec![GovernanceEntry::JoinRequested {
                requester: joiner.clone(),
                display_name: "also-forged".into(),
                lamport: 4,
            }],
        ),
    ]);

    assert!(
        state.pending_members.is_empty(),
        "a JoinRequested whose author is not the requester must be dropped — \
         otherwise anyone could fill a moderator's queue with fabricated names"
    );
}

#[test]
fn self_authored_join_request_is_recorded_then_cleared_by_approval() {
    let creator = pseudo(1);
    let joiner = pseudo(2);

    let state = merge(&[
        (creator.clone(), genesis()),
        (
            joiner.clone(),
            vec![GovernanceEntry::JoinRequested {
                requester: joiner.clone(),
                display_name: "ada".into(),
                lamport: 3,
            }],
        ),
    ]);
    assert!(state.pending_members.contains_key(&joiner));

    let mut with_decision = genesis();
    with_decision.push(GovernanceEntry::MemberApproved {
        target: joiner.clone(),
        lamport: 4,
    });
    let state = merge(&[
        (creator, with_decision),
        (
            joiner.clone(),
            vec![GovernanceEntry::JoinRequested {
                requester: joiner.clone(),
                display_name: "ada".into(),
                lamport: 3,
            }],
        ),
    ]);
    assert!(
        !state.pending_members.contains_key(&joiner),
        "an approval clears the pending row"
    );
    assert!(state.admitted[&joiner].approved);
}

#[test]
fn a_settled_request_is_not_resurrected_by_a_replayed_join() {
    // Delivery order is not guaranteed: the decision may merge before
    // the request it answers. Convergence requires the late request to
    // lose, not to reopen a closed case.
    let creator = pseudo(1);
    let joiner = pseudo(2);

    let mut creator_entries = genesis();
    creator_entries.push(GovernanceEntry::MemberRejected {
        target: joiner.clone(),
        reason: Some("no".into()),
        lamport: 3,
    });

    let state = merge(&[
        (creator, creator_entries),
        (
            joiner.clone(),
            vec![GovernanceEntry::JoinRequested {
                requester: joiner.clone(),
                display_name: "ada".into(),
                lamport: 9, // arrives "later" than the rejection
            }],
        ),
    ]);
    assert!(
        state.pending_members.is_empty(),
        "a replayed request must not reopen a settled decision"
    );
    assert!(!state.admitted[&joiner].approved);
}

#[test]
fn admission_mode_defaults_to_open() {
    let state = merge(&[(pseudo(1), genesis())]);
    assert_eq!(state.effective_admission_mode(), AdmissionMode::Open);
    assert!(
        state.admission_mode.is_none(),
        "unset is distinct from explicitly Open"
    );
}

#[test]
fn admission_mode_is_settable_once_by_the_creator() {
    let creator = pseudo(1);
    let mut entries = genesis();
    entries.push(GovernanceEntry::AdmissionPolicy {
        mode: AdmissionMode::ApprovalRequired,
        lamport: 3,
    });
    // A second policy entry, later, by the same creator.
    entries.push(GovernanceEntry::AdmissionPolicy {
        mode: AdmissionMode::Open,
        lamport: 4,
    });

    let state = merge(&[(creator, entries)]);
    assert_eq!(
        state.effective_admission_mode(),
        AdmissionMode::ApprovalRequired,
        "the second AdmissionPolicy must be ignored — that is what makes the \
         mode immutable, and why it does not live on CommunityPolicy"
    );
}

#[test]
fn a_non_creator_cannot_set_or_flip_the_admission_mode() {
    let creator = pseudo(1);
    let admin = pseudo(2);

    let mut creator_entries = genesis();
    creator_entries.push(GovernanceEntry::AdmissionPolicy {
        mode: AdmissionMode::ApprovalRequired,
        lamport: 3,
    });
    // An all-permissions member is still not the creator.
    creator_entries.push(GovernanceEntry::RoleAssignment {
        target: admin.clone(),
        role_id: role_id(0),
        lamport: 4,
    });

    let state = merge(&[
        (creator, creator_entries),
        (
            admin,
            vec![GovernanceEntry::AdmissionPolicy {
                mode: AdmissionMode::Open,
                lamport: 5,
            }],
        ),
    ]);
    assert_eq!(
        state.effective_admission_mode(),
        AdmissionMode::ApprovalRequired,
        "MANAGE_COMMUNITY must not be able to open a private community"
    );
}

#[test]
fn the_creator_cannot_exceed_the_invite_use_cap() {
    // `invite_quota` documents this as holding "even [for] a
    // creator-bypass writer". It did not: the cap was checked inside the
    // permission match, below the `creator always passes` short-circuit,
    // so the creator could store u32::MAX. The admission design leans on
    // this bound for slot exhaustion, so it must bind everyone.
    let creator = pseudo(1);
    let mut entries = genesis();
    entries.push(GovernanceEntry::InviteCreated {
        invite_id: [9u8; 16],
        code_hash: "abc".into(),
        max_uses: u32::MAX,
        expires_at: None,
        secrets_record_key: "VLD0:rec".into(),
        lamport: 3,
    });
    let state = merge(&[(creator, entries)]);
    assert!(
        state.invites.is_empty(),
        "an over-cap invite must be excluded from merged state, creator or not"
    );
}
