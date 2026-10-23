//! Reader-validated bounds (plan E3.0): an entry outside its value bounds
//! is dropped by every reader, whoever wrote it and wherever it sorts.

use super::*;
use crate::fixture::{cert, ops_in_order};
use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
use rekindle_types::governance::AdmissionMode;
use rekindle_types::key_format::MAX_DESCRIPTION_LEN;
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

fn meta(description: Option<String>, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description,
            icon_hash: None,
            banner_hash: None,
        },
    )
}

fn role_all(id: u8, position: u32, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::RoleDefinition {
            role_id: role_id(id),
            name: "all".into(),
            permissions: rekindle_types::permissions::ALL,
            position,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
    )
}

fn channel(lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::ChannelCreated {
            channel_id: channel_id(7),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: "k".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
        },
    )
}

fn slowmode(seconds: u32, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::ChannelUpdated {
            channel_id: channel_id(7),
            name: None,
            topic: None,
            forum_tags: None,
            position: None,
            slowmode_seconds: Some(seconds),
            nsfw: None,
            category_id: None,
        },
    )
}

fn timeout(target: PseudonymKey, duration_seconds: u64, lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::TimeoutEntry {
            target,
            duration_seconds,
            reason: None,
            started_at: 1_000,
        },
    )
}

/// The creator's base: meta, an everyone-ALL role, a ranked ALL role held by
/// the peer `pseudo(2)`, and one channel, so the peer passes every
/// permission and rank check and only the bound decides.
fn base() -> Vec<Stamped> {
    vec![
        meta(None, 1),
        role_all(0, 0, 2),
        role_all(1, 5, 3),
        at(
            4,
            GovernanceEntry::RoleAssignment {
                target: pseudo(2),
                role_id: role_id(1),
            },
        ),
        channel(5),
    ]
}

fn slowmode_of(state: &GovernanceState) -> Option<u32> {
    state.channels[&channel_id(7)].slowmode_seconds
}

#[test]
fn slowmode_over_the_bound_is_dropped_from_peer_and_creator() {
    let (creator, peer) = (pseudo(1), pseudo(2));
    for writer in [&creator, &peer] {
        let mut subkeys = vec![(creator.clone(), base())];
        let over = slowmode(MAX_SLOWMODE_SECONDS + 1, 10);
        if *writer == creator {
            subkeys[0].1.push(over);
        } else {
            subkeys.push((peer.clone(), vec![over]));
        }
        assert_eq!(slowmode_of(&merge_entries(&creator, &subkeys)), None);
    }
    let at_bound = merge_entries(
        &creator,
        &[
            (creator.clone(), base()),
            (peer, vec![slowmode(MAX_SLOWMODE_SECONDS, 10)]),
        ],
    );
    assert_eq!(slowmode_of(&at_bound), Some(MAX_SLOWMODE_SECONDS));
}

#[test]
fn timeout_outside_the_range_is_dropped_from_peer_and_creator() {
    let (creator, peer, target) = (pseudo(1), pseudo(2), pseudo(3));
    for duration in [0, MAX_TIMEOUT_SECONDS + 1] {
        let state = merge_entries(
            &creator,
            &[
                (
                    creator.clone(),
                    [base(), vec![timeout(target.clone(), duration, 10)]].concat(),
                ),
                (peer.clone(), vec![timeout(target.clone(), duration, 11)]),
            ],
        );
        assert!(!state.timeouts.contains_key(&target), "{duration} s");
    }
    let state = merge_entries(
        &creator,
        &[
            (creator.clone(), base()),
            (peer, vec![timeout(target.clone(), MAX_TIMEOUT_SECONDS, 10)]),
        ],
    );
    assert_eq!(
        state.timeouts[&target].duration_seconds,
        MAX_TIMEOUT_SECONDS
    );
}

#[test]
fn description_the_ipc_refuses_is_dropped_from_peer_and_creator() {
    let (creator, peer) = (pseudo(1), pseudo(2));
    for bad in ["x".repeat(MAX_DESCRIPTION_LEN + 1), "bell\u{7}".into()] {
        let state = merge_entries(
            &creator,
            &[
                (
                    creator.clone(),
                    [base(), vec![meta(Some(bad.clone()), 10)]].concat(),
                ),
                (peer.clone(), vec![meta(Some(bad), 11)]),
            ],
        );
        assert_eq!(state.metadata.expect("base meta").stamp.lamport, 1);
    }
    let fits = "é".repeat(MAX_DESCRIPTION_LEN);
    let state = merge_entries(
        &creator,
        &[
            (creator.clone(), base()),
            (peer, vec![meta(Some(fits.clone()), 10)]),
        ],
    );
    assert_eq!(state.metadata.expect("meta").description, Some(fits));
}

/// The lowest-lamport entry is the genesis and is applied unvalidated, so an
/// out-of-bounds one must neither apply nor make its author the creator.
#[test]
fn out_of_bounds_lowest_lamport_entry_is_not_the_genesis() {
    let (attacker, creator, target) = (pseudo(9), pseudo(1), pseudo(3));
    let genesis_attempts = [
        meta(Some("x".repeat(MAX_DESCRIPTION_LEN + 1)), 0),
        timeout(target.clone(), 0, 0),
        slowmode(u32::MAX, 0),
    ];
    for attempt in genesis_attempts {
        let ops = ops_in_order(&[(attacker.clone(), vec![attempt]), (creator.clone(), base())]);
        let state = merge(&cert(&creator, AdmissionMode::default()), &ops);
        assert_eq!(state.creator, Some(creator.clone()));
        let creators_meta = ops
            .iter()
            .find(|o| {
                o.author() == &creator && matches!(o.body(), GovernanceEntry::CommunityMeta { .. })
            })
            .expect("the creator's meta op");
        assert_eq!(
            state.metadata.as_ref().expect("meta").stamp,
            creators_meta.stamp()
        );
        assert!(state.timeouts.is_empty());
        assert_eq!(slowmode_of(&state), None);
    }
}
