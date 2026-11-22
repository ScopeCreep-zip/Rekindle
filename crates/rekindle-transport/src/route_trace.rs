//! Route blob trace lines (plan RD2): one INFO line when we publish our own
//! route blob and one when we import a peer's blob for a call, so a test
//! run whose processes share `REKINDLE_LOG_SCRUB_KEY` (debug builds,
//! `rekindle_utils::log_scrub`) can match the publisher's blob to the one
//! its peers send to, and tell a stale blob from a route unreachable from
//! outside. Every identifier is already the scrubber's tag here: the peer
//! and the route id as the sink would tag them on any other line, the blob
//! as the tag of its BLAKE3; no key or blob byte enters the event.

use rekindle_records::port::{PeerTarget, RouteClass, RoutePort};
use rekindle_utils::log_scrub::{scrub, tag};

/// The tag of `blob`: the scrubber's tag of its BLAKE3.
#[must_use]
pub fn blob_tag(blob: &[u8]) -> String {
    tag(&rekindle_utils::blake3_hex(blob))
}

/// The id of our `class` route while `blob` is still its blob; `None`
/// once a newer route replaced it.
#[must_use]
pub fn own_route_id(routes: &dyn RoutePort, class: RouteClass, blob: &[u8]) -> Option<String> {
    let id = routes.own_id(class)?;
    (routes.own_blob(class).as_deref() == Some(blob)).then_some(id)
}

/// We wrote our `class` route blob to the records peers read it from
/// (`landed`: every write reached consensus).
pub fn published(class: RouteClass, route_id: Option<&str>, blob: &[u8], landed: bool) {
    let route = route_id.map_or_else(|| "replaced".to_owned(), |id| scrub(id).into_owned());
    tracing::info!(
        ?class,
        route = %route,
        blob = %blob_tag(blob),
        blob_len = blob.len(),
        landed,
        "own route blob published"
    );
}

/// We imported `peer`'s route `blob` as `target` to call them.
pub fn imported(peer: &str, target: &PeerTarget, blob: &[u8]) {
    tracing::info!(
        peer = %scrub(peer),
        route = %scrub(target.as_str()),
        blob = %blob_tag(blob),
        blob_len = blob.len(),
        "peer route blob imported for a call"
    );
}

#[cfg(test)]
#[path = "route_trace_tests.rs"]
mod tests;
