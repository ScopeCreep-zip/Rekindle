//! The own-record watch tells the user once that our key was retired
//! (plan E3.2 S7d-keys.9).

use super::first_retired_notice;
use crate::host::community::state::CommunityState;

/// The first retirement the watch finds is told; every later change that
/// finds it again is not.
#[test]
fn a_retirement_is_told_once_a_session() {
    let mut community = CommunityState::default();
    assert!(first_retired_notice(&mut community), "the first is told");
    assert!(
        !first_retired_notice(&mut community),
        "a later change finding it again is not"
    );
}
