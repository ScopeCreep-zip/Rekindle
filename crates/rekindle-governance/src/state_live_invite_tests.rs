//! `GovernanceState::live_invite` (plan E3.2 S7c.5): what a join and a
//! joiner's pre-join snapshot request are checked against.

use super::{GovernanceState, InviteState};
use crate::op::OpStamp;
use rekindle_types::id::PseudonymKey;

fn with_invite(uses: u32, max_uses: u32, expires_at: Option<u64>) -> GovernanceState {
    let mut state = GovernanceState::default();
    state.invites.insert(
        [1; 16],
        InviteState {
            code_hash: "code".into(),
            max_uses,
            expires_at,
            secrets_record_key: "VLD0:s".into(),
            created: OpStamp::MIN,
            creator_pseudonym: PseudonymKey([2; 32]),
            uses,
        },
    );
    state
}

#[test]
fn a_used_up_expired_or_unknown_invite_is_not_live() {
    assert!(with_invite(1, 2, None).live_invite("code", 10).is_some());
    assert!(
        with_invite(2, 2, None).live_invite("code", 10).is_none(),
        "used up"
    );
    assert!(
        with_invite(0, 2, Some(10))
            .live_invite("code", 10)
            .is_none(),
        "expired"
    );
    assert!(
        with_invite(0, 2, None).live_invite("other", 10).is_none(),
        "unknown"
    );
}
