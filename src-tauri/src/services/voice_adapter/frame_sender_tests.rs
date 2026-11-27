use std::cell::Cell;

use veilid_core::VeilidAPIError;

use super::forget_if_peer_side;

fn forgets(e: &VeilidAPIError) -> bool {
    let forgot = Cell::new(false);
    forget_if_peer_side(e, || forgot.set(true));
    forgot.get()
}

/// Our released safety route at teardown answers `InvalidTarget`, our
/// first hop down `NoConnection` with another text: the peer's route
/// stays imported (live finding CR3).
#[test]
fn our_side_failure_at_teardown_keeps_the_peer_route() {
    assert!(!forgets(&VeilidAPIError::invalid_target("route released")));
    assert!(!forgets(&VeilidAPIError::no_connection(
        "no routing domain"
    )));
}

/// The peer's route not in Veilid's remote cache is theirs: forgotten.
#[test]
fn an_unresolved_remote_route_is_forgotten() {
    assert!(forgets(&VeilidAPIError::no_connection(
        "could not get remote private route"
    )));
}
