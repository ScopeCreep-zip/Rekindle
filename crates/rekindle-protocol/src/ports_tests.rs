//! Tests for the Veilid ports: the peer-route and send error mapping, the
//! update translation and detach (moved out of `ports.rs`, file ceiling).

use rekindle_types::notification::AttachmentState;

/// Only the peer's unresolved route makes it `RouteUnusable`; a
/// `NoConnection` or `InvalidTarget` on our side keeps the peer's route
/// (plan VU5, synthesis #9).
#[test]
fn only_the_peers_unresolved_route_is_route_unusable() {
    use rekindle_records::port::PortError;
    use veilid_core::VeilidAPIError;

    let peer = VeilidAPIError::no_connection(super::REMOTE_ROUTE_UNRESOLVED);
    assert!(super::peer_route_unusable(&peer));
    assert!(matches!(
        super::send_error(&peer),
        PortError::RouteUnusable(_)
    ));
    for ours in [
        VeilidAPIError::no_connection("no routing domain"),
        VeilidAPIError::no_connection("No peer info for stub private route"),
        VeilidAPIError::invalid_target("Own node info must be published to use private route"),
    ] {
        assert!(!super::peer_route_unusable(&ours), "{ours}");
        assert!(
            matches!(super::send_error(&ours), PortError::Send(_)),
            "{ours}"
        );
    }
    // A lost question (the reply deadline elapsed, as a call into a
    // released route ends) keeps its own type, so a member call
    // re-reads `M`/6 after it (plan E3.4 S4).
    assert!(matches!(
        super::send_error(&VeilidAPIError::timeout()),
        PortError::Timeout(_)
    ));
}

/// Every attached state veilid-core can emit maps to an attached state
/// on our side, through the string [`super::to_port_event`] carries.
///
/// `from_veilid_string` fails closed to `Detached`, so an upstream
/// rename or a new variant does not break the build — it silently
/// reports a healthy node as detached. veilid-core 0.5.4 did both at
/// once (added `attached_fair`, renamed `FullyAttached` ->
/// `AttachedFull`) and the changelog mentioned neither. This walks
/// upstream's own enum so the next one fails here instead. It sits
/// beside the mapping, not beside the enum in `rekindle-types`: Tier 1
/// must not depend on veilid-core (rule B2).
#[test]
fn every_veilid_attachment_string_is_understood() {
    use veilid_core::AttachmentState as Upstream;

    for upstream in [
        Upstream::Detached,
        Upstream::Attaching,
        Upstream::AttachedWeak,
        Upstream::AttachedFair,
        Upstream::AttachedGood,
        Upstream::AttachedStrong,
        Upstream::AttachedFull,
        Upstream::Detaching,
    ] {
        let rendered = upstream.to_string();
        let ours = AttachmentState::from_veilid_string(&rendered);

        assert_eq!(
            ours.is_attached(),
            upstream.is_attached(),
            "{rendered:?} parsed to {ours:?}, whose is_attached() \
             disagrees with veilid-core"
        );

        if !matches!(upstream, Upstream::Detached) {
            assert_ne!(
                ours,
                AttachmentState::Detached,
                "{rendered:?} fell through to the fail-closed Detached \
                 arm — add it to from_veilid_string"
            );
        }
    }
}

/// A detach of a node already detached is success; any other refusal is
/// the port's error (plan VU4, synthesis #12).
#[test]
fn not_attached_is_a_successful_detach() {
    use veilid_core::VeilidAPIError;
    assert!(super::detach_result(Ok(())).is_ok());
    assert!(super::detach_result(Err(VeilidAPIError::generic("Not attached"))).is_ok());
    assert!(super::detach_result(Err(VeilidAPIError::not_initialized())).is_err());
    assert!(super::detach_result(Err(VeilidAPIError::generic("other"))).is_err());
}
