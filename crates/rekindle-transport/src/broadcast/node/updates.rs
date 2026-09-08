//! Veilid update classification helpers — used by the dispatch bootstrap
//! (`veilid_update_label`) and the route authority loop
//! (`personal_route_died`).

/// Human-readable `VeilidUpdate` variant name for logs.
///
/// Re-exported from `rekindle-protocol` rather than re-matched: the
/// arms must track veilid's enum, and two copies drift the moment
/// upstream adds a variant.
pub(super) use rekindle_protocol::node::veilid_update_name as veilid_update_label;

/// Did our personal route die in this batch?
///
/// This used to also report which community mailboxes lost their
/// published route. Flat governance has no community mailbox — a member
/// is reached through the route blob in its own registry row — so the
/// personal route is the only one this node publishes.
pub(super) fn personal_route_died(
    dead: &[veilid_core::RouteId],
    personal: Option<&veilid_core::RouteId>,
) -> bool {
    personal.is_some_and(|p| dead.contains(p))
}

#[cfg(test)]
mod route_authority_tests {
    use super::personal_route_died;

    fn route_id(byte: u8) -> veilid_core::RouteId {
        veilid_core::RouteId::new(
            veilid_core::CRYPTO_KIND_VLD0,
            veilid_core::BareRouteId::new(&[byte; 32]),
        )
    }

    #[test]
    fn our_route_in_the_dead_batch() {
        let personal = route_id(1);
        assert!(personal_route_died(&[route_id(1)], Some(&personal)));
    }

    #[test]
    fn someone_elses_dead_route_is_not_ours() {
        let personal = route_id(1);
        assert!(!personal_route_died(&[route_id(9)], Some(&personal)));
    }

    /// No personal route allocated yet — nothing of ours can have died,
    /// so the heal path must not fire on another node's churn.
    #[test]
    fn no_personal_route_never_dies() {
        assert!(!personal_route_died(&[route_id(2), route_id(3)], None));
    }
}
