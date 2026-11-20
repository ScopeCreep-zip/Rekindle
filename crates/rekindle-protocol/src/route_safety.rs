//! The safety selection of a send pinned to one of our own routes: every
//! app message (statement) to our `Sending` route, as every 1:1 call
//! already is (plan RH1r, E2.1).

use rekindle_records::port::{PortError, RouteClass};
use rekindle_types::config::SafetyProfile;
use veilid_core::RouteId;

use crate::dht::pool::safety_selection;

/// The route class every app message (a statement) is pinned to: our
/// tested `Sending` route, the one our 1:1 calls are already pinned to
/// (plan RH1r, E2.1). Unpinned, Veilid picks an automatic pool route
/// (0.5.8 `route_select.rs:182-188`, `:265-269`), which may be untested or
/// failing its loopback tests, and a statement lost on it vanishes: Veilid
/// counts only lost questions against a route (`route_stats.rs:144-147`,
/// `:409`) and demotes a route on loss only for questions
/// (`route_select.rs:321-410`). A statement names no reply route
/// (`rpc_processor/mod.rs:1060-1063`), so the recipient learns nothing of
/// the pinned safety route (threat model L2).
pub(crate) const STATEMENT_ROUTE: RouteClass = RouteClass::Sending;

/// The safety selection of a send pinned to our live route `own` of
/// `class` (`SafetySpec.preferred_route`, 0.5.8 `safety.rs:273`, taken
/// first by `route_select.rs:148-180`); `Routing` while none is live,
/// never a pool route in its place.
pub(crate) fn pinned_safety(
    profile: &SafetyProfile,
    class: RouteClass,
    own: Option<RouteId>,
) -> Result<veilid_core::SafetySelection, PortError> {
    let id =
        own.ok_or_else(|| PortError::Routing(format!("our {class:?} route is not available")))?;
    let veilid_core::SafetySelection::Safe(mut spec) = safety_selection(profile) else {
        return Err(PortError::Routing("safety selection is not Safe".into()));
    };
    spec.preferred_route = Some(id);
    Ok(veilid_core::SafetySelection::Safe(spec))
}

/// The safety selection of an app message: pinned to our live
/// [`STATEMENT_ROUTE`], whose id `own` answers by class.
pub(crate) fn statement_safety(
    profile: &SafetyProfile,
    own: impl Fn(RouteClass) -> Option<RouteId>,
) -> Result<veilid_core::SafetySelection, PortError> {
    pinned_safety(profile, STATEMENT_ROUTE, own(STATEMENT_ROUTE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::config::{SequencingPreference, StabilityPreference};
    use veilid_core::{BareRouteId, SafetySelection, CRYPTO_KIND_VLD0};

    fn route(fill: u8) -> RouteId {
        RouteId::new(CRYPTO_KIND_VLD0, BareRouteId::new(&[fill; 32]))
    }

    fn profile() -> SafetyProfile {
        SafetyProfile {
            hop_count: 3,
            stability: StabilityPreference::Reliable,
            sequencing: SequencingPreference::NoPreference,
        }
    }

    /// An app message is pinned to our `Sending` route, the one our 1:1
    /// calls take, never another class and never left to Veilid's pool.
    #[test]
    fn a_statement_is_pinned_to_our_sending_route() {
        let own = |class: RouteClass| match class {
            RouteClass::Sending => Some(route(7)),
            RouteClass::General => Some(route(1)),
            RouteClass::Media => Some(route(2)),
        };
        let Ok(SafetySelection::Safe(spec)) = statement_safety(&profile(), own) else {
            panic!("a statement's selection is Safe");
        };
        assert_eq!(spec.preferred_route, Some(route(7)));
        assert!(spec.hop_count >= 3, "the anonymity floor holds");
    }

    /// While our `Sending` route is not live a statement is refused with
    /// `Routing`; no pool route is taken in its place.
    #[test]
    fn no_sending_route_refuses_the_statement() {
        let own = |class: RouteClass| (class != RouteClass::Sending).then(|| route(1));
        assert!(matches!(
            statement_safety(&profile(), own),
            Err(PortError::Routing(_))
        ));
    }
}
