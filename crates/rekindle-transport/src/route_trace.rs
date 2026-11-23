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

/// The scrubbed `route` and `blob` fields of a line about a call to
/// `target`, imported from `blob`: the same tags on every call line, so
/// one route instance matches across them and the publisher's line.
fn call_route(target: &PeerTarget, blob: &[u8]) -> (String, String) {
    (scrub(target.as_str()).into_owned(), blob_tag(blob))
}

/// We imported `peer`'s route `blob` as `target` to call them.
pub fn imported(peer: &str, target: &PeerTarget, blob: &[u8]) {
    let (route, blob_tag) = call_route(target, blob);
    tracing::info!(
        peer = %scrub(peer),
        route = %route,
        blob = %blob_tag,
        blob_len = blob.len(),
        "peer route blob imported for a call"
    );
}

/// We `app_call` a bare community envelope (the join's bootstrap
/// exchange, a member call, a MEK delivery) to `target`, imported from
/// `blob`, over a safety route of `hops` hops (live finding AB1).
pub fn envelope_call_sent(target: &PeerTarget, blob: &[u8], hops: u8) {
    let (route, blob_tag) = call_route(target, blob);
    tracing::info!(
        route = %route,
        blob = %blob_tag,
        blob_len = blob.len(),
        hops,
        "community-envelope RPC sent"
    );
}

/// The community-envelope call to `target` (imported from `blob`, over
/// `hops` hops) got no answer before its deadline.
pub fn envelope_call_timed_out(target: &PeerTarget, blob: &[u8], hops: u8, reason: &str) {
    let (route, blob_tag) = call_route(target, blob);
    tracing::info!(
        route = %route,
        blob = %blob_tag,
        blob_len = blob.len(),
        hops,
        reason,
        "community-envelope RPC timed out"
    );
}

#[cfg(test)]
#[path = "route_trace_tests.rs"]
mod tests;
