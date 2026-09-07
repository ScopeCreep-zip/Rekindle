//! Veilid update classification helpers — used by the dispatch bootstrap
//! (`veilid_update_label`) and the route authority loop
//! (`classify_dead_routes`).

/// Human-readable `VeilidUpdate` variant name for logs.
///
/// Re-exported from `rekindle-protocol` rather than re-matched: the
/// arms must track veilid's enum, and two copies drift the moment
/// upstream adds a variant.
pub(super) use rekindle_protocol::node::veilid_update_name as veilid_update_label;

/// Pure classification of a dead-route batch against owned state:
/// did the personal route die, and which community mailboxes lost
/// their published route.
pub(super) fn classify_dead_routes(
    dead: &[veilid_core::RouteId],
    personal: Option<&veilid_core::RouteId>,
    community_live: &std::collections::HashMap<String, veilid_core::RouteId>,
) -> (bool, Vec<String>) {
    let personal_died = personal.is_some_and(|p| dead.contains(p));
    let mailboxes = community_live
        .iter()
        .filter(|(_, id)| dead.contains(id))
        .map(|(key, _)| key.clone())
        .collect();
    (personal_died, mailboxes)
}

#[cfg(test)]
mod route_authority_tests {
    use super::classify_dead_routes;

    fn route_id(byte: u8) -> veilid_core::RouteId {
        veilid_core::RouteId::new(
            veilid_core::CRYPTO_KIND_VLD0,
            veilid_core::BareRouteId::new(&[byte; 32]),
        )
    }

    #[test]
    fn classify_dead_routes_personal() {
        let personal = route_id(1);
        let dead = vec![route_id(1)];
        let live = std::collections::HashMap::new();
        let (personal_died, mailboxes) = classify_dead_routes(&dead, Some(&personal), &live);
        assert!(personal_died);
        assert!(mailboxes.is_empty());
    }

    #[test]
    fn classify_dead_routes_community() {
        let dead = vec![route_id(2)];
        let mut live = std::collections::HashMap::new();
        live.insert("mb1".to_string(), route_id(2));
        live.insert("mb2".to_string(), route_id(3));
        let (personal_died, mailboxes) = classify_dead_routes(&dead, None, &live);
        assert!(!personal_died);
        assert_eq!(mailboxes, vec!["mb1".to_string()]);
    }

    #[test]
    fn classify_dead_routes_none() {
        let personal = route_id(1);
        let dead = vec![route_id(9)];
        let live = std::collections::HashMap::new();
        let (personal_died, mailboxes) = classify_dead_routes(&dead, Some(&personal), &live);
        assert!(!personal_died);
        assert!(mailboxes.is_empty());
    }
}
