//! Peer-assisted join control-message handler (drained from
//! `legacy/onboarding.rs::handle_peer_assisted_join`).

use crate::membership_events::deps::MembershipEventDeps;

/// Note a peer's join: add them to `known_members` and, when the request
/// carries a redeemable invite code, bump our local uses counter.
pub fn process_peer_assisted_join<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    pseudonym_key: &str,
    display_name: &str,
    claimed_subkey_index: Option<u32>,
    route_blob: Option<&[u8]>,
    invite_code: Option<&str>,
) {
    deps.insert_known_members(community_id, &[pseudonym_key.to_string()]);
    tracing::info!(
        community = %community_id,
        pseudonym = %pseudonym_key,
        "peer join noted — added to known_members"
    );

    if let Some(code) = invite_code {
        deps.bump_invite_uses(community_id, code);
    }

    let _ = (display_name, claimed_subkey_index, route_blob);
}
