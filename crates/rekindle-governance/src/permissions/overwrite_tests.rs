//! Channel overwrites written as the API sends them (a role's 32 hex, a
//! member's 64 hex) reach `compute_permissions` and `channel_read_roster`
//! through the merge: `@everyone` and role overwrites apply, not only
//! member ones.

use std::collections::{HashMap, HashSet};

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{DEFAULT_EVERYONE, READ_HISTORY, SEND_MESSAGES, VIEW_CHANNELS};

use super::{channel_read_roster, compute_permissions};
use crate::merge::apply_entry;
use crate::state::{GovernanceState, RoleState, TimeoutState};
use crate::validate::validate_write;

const RESTRICTED_CHANNEL: ChannelId = ChannelId([9; 16]);
const OPEN: ChannelId = ChannelId([8; 16]);
const MUTED: RoleId = RoleId([5; 16]);

fn key(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn role(permissions: u64) -> RoleState {
    RoleState {
        name: "r".into(),
        permissions,
        position: 1,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
        stamp: crate::fixture::stamp(1),
    }
}

/// Creator `key(1)`; plain members `key(2)`, `key(3)`; `key(4)` holds `MUTED`.
fn state() -> GovernanceState {
    let mut state = GovernanceState {
        creators: std::collections::BTreeSet::from([key(1)]),
        roles: HashMap::from([(RoleId::EVERYONE, role(DEFAULT_EVERYONE)), (MUTED, role(0))]),
        ..GovernanceState::default()
    };
    state
        .role_assignments
        .insert(key(4), HashSet::from([MUTED]));
    state
}

/// The overwrite exactly as IPC and the governance op carry it.
fn overwrite(target_type: &str, target_id: String, allow: u64, deny: u64) -> GovernanceEntry {
    GovernanceEntry::PermissionOverwrite {
        channel_id: RESTRICTED_CHANNEL,
        target_type: target_type.into(),
        target_id,
        allow,
        deny,
    }
}

fn write(state: &mut GovernanceState, entry: &GovernanceEntry, lamport: u64) {
    assert!(
        validate_write(&key(1), entry, state),
        "the creator may write {entry:?}"
    );
    apply_entry(&key(1), entry, crate::fixture::stamp(lamport), state);
}

fn sees(state: &GovernanceState, member: u8, channel: ChannelId) -> bool {
    compute_permissions(&key(member), Some(&channel), state, 0) & VIEW_CHANNELS != 0
}

#[test]
fn an_everyone_deny_hides_a_channel_and_a_member_allow_restores_it_for_that_member_only() {
    let mut state = state();
    assert!(
        sees(&state, 2, RESTRICTED_CHANNEL),
        "an open channel before any overwrite"
    );
    write(
        &mut state,
        &overwrite("role", RoleId::EVERYONE.to_hex(), 0, VIEW_CHANNELS),
        2,
    );
    assert!(
        !sees(&state, 2, RESTRICTED_CHANNEL),
        "@everyone denied VIEW hides it from a plain member"
    );
    assert!(sees(&state, 2, OPEN), "the deny is on one channel only");
    write(
        &mut state,
        &overwrite("member", key(3).to_hex(), VIEW_CHANNELS, 0),
        3,
    );
    assert!(
        sees(&state, 3, RESTRICTED_CHANNEL),
        "the allowed member sees it again"
    );
    assert!(
        !sees(&state, 2, RESTRICTED_CHANNEL),
        "the allow is that member's only"
    );

    let members = [key(1), key(2), key(3), key(4)];
    assert_eq!(
        channel_read_roster(&state, &RESTRICTED_CHANNEL, &members),
        vec![key(1), key(3)],
        "the channel is restricted: its readers are the creator and the allowed member"
    );
    assert_eq!(
        channel_read_roster(&state, &OPEN, &members),
        members.to_vec()
    );
}

#[test]
fn a_role_deny_applies_to_its_holders_only() {
    let mut state = state();
    write(
        &mut state,
        &overwrite("role", MUTED.to_hex(), 0, SEND_MESSAGES),
        2,
    );
    let perms = |m: u8| compute_permissions(&key(m), Some(&RESTRICTED_CHANNEL), &state, 0);
    assert_eq!(
        perms(4) & SEND_MESSAGES,
        0,
        "the role's holder may not send"
    );
    assert_ne!(
        perms(2) & SEND_MESSAGES,
        0,
        "a member without the role still sends"
    );
}

#[test]
fn a_malformed_target_is_refused_and_never_stored() {
    let mut state = state();
    let debug = overwrite("role", format!("{:?}", RoleId::EVERYONE), 0, VIEW_CHANNELS);
    assert!(!validate_write(
        &key(1),
        &overwrite("role", "general".into(), 0, 0),
        &state
    ));
    assert!(!validate_write(&key(1), &debug, &state));
    apply_entry(&key(1), &debug, crate::fixture::stamp(2), &mut state);
    assert!(
        state.overwrites.is_empty(),
        "the merge drops a target that is not canonical"
    );
}

/// `key(2)` timed out from 1000 for an hour.
fn timed_out(state: &mut GovernanceState) {
    state.timeouts.insert(
        key(2),
        TimeoutState {
            duration_seconds: 3600,
            started_at: 1000,
            reason: None,
            stamp: crate::fixture::stamp(3),
        },
    );
}

#[test]
fn a_timeout_never_reopens_a_channel_an_overwrite_closed() {
    let mut state = state();
    write(
        &mut state,
        &overwrite("role", RoleId::EVERYONE.to_hex(), 0, VIEW_CHANNELS),
        1,
    );
    timed_out(&mut state);
    let perms = compute_permissions(&key(2), Some(&RESTRICTED_CHANNEL), &state, 1500);
    assert_eq!(
        perms & VIEW_CHANNELS,
        0,
        "timed out on a denied channel: {perms:#x}"
    );
    assert_eq!(
        perms & READ_HISTORY,
        0,
        "timed out on a denied channel: {perms:#x}"
    );
}

#[test]
fn a_timeout_on_an_open_channel_keeps_view_and_history_and_takes_send() {
    let mut state = state();
    timed_out(&mut state);
    let perms = compute_permissions(&key(2), Some(&OPEN), &state, 1500);
    assert_eq!(perms, VIEW_CHANNELS | READ_HISTORY, "{perms:#x}");
    assert_eq!(perms & SEND_MESSAGES, 0);
    // Expired: the member's own permissions come back.
    let perms = compute_permissions(&key(2), Some(&OPEN), &state, 5000);
    assert_ne!(perms & SEND_MESSAGES, 0);
}

#[test]
fn a_timeout_never_grants_history_the_member_lacked() {
    let mut state = state();
    // `@everyone` may view but not read history on the open channel.
    state
        .roles
        .insert(RoleId::EVERYONE, role(VIEW_CHANNELS | SEND_MESSAGES));
    timed_out(&mut state);
    let perms = compute_permissions(&key(2), Some(&OPEN), &state, 1500);
    assert_eq!(perms, VIEW_CHANNELS, "{perms:#x}");
}
