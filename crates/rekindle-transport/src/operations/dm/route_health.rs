//! A peer's route health around each 1:1 fast-path send (plan RH1r-c
//! S1–S6; `crate::liveness::peer`): count the sends that expect an
//! answer, run a suspect round when one is due, and hold the fast path back
//! while the peer is unreachable. The durable chain (the outbox's appends)
//! is never gated here: it is the guaranteed path, the route only the fast
//! one (S6).

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_route::dead_routes::DeadRouteVerdict;
use rekindle_route::liveness::Fetched;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::route;
use crate::broadcast::node::TransportNode;
use crate::liveness::dead::Probed;
use crate::operations::friend::FriendHost;

/// Whether a send of `kind` is answered by the peer (a `DmAck`, a reply or
/// a receipt), so its silence counts against their route (S1): not an ack
/// itself, nor an ephemeral frame.
fn expects_answer(kind: DmTypeId) -> bool {
    kind != DmTypeId::DmAck && !kind.is_ephemeral()
}

/// Before a fast-path send to `to`: run a suspect round when one is due
/// (S2, S3). `false` while the peer is unreachable and its backoff step is
/// not up (S4).
pub(crate) async fn before_send(host: &dyn FriendHost, to: &str) -> bool {
    let Ok(node) = host.node() else {
        return true;
    };
    if node.liveness().round_due(to) {
        suspect_round(host, &node, to).await;
    }
    node.liveness().fast_path_allowed(to)
}

/// A send of `kind` to `to` left over the fast path.
pub(crate) fn sent(host: &dyn FriendHost, to: &str, kind: DmTypeId) {
    if let (true, Ok(node)) = (expects_answer(kind), host.node()) {
        node.liveness().sent(to);
    }
}

/// An authenticated frame from `peer` arrived (S5).
pub(crate) fn heard(host: &dyn FriendHost, peer: &str) {
    if let Ok(node) = host.node() {
        if node.liveness().heard(peer) {
            reachability(host, peer, true);
        }
    }
}

fn reachability(host: &dyn FriendHost, peer: &str, reachable: bool) {
    tracing::info!(peer = %peer, reachable, "peer route reachability changed");
    host.publish(SubscriptionEvent::Friend(FriendEvent::Reachability {
        peer_key: peer.to_string(),
        reachable,
    }));
}

/// One suspect round (S2): re-read the peer's route record; a strictly
/// higher seq replaces the cached route; the record we hold is probed once,
/// beside the send that ran the round (A/B round 5 finding R5-1): the
/// probe's answer never changes which route a hint takes, so the hint does
/// not wait the probe's round trip, or our `app_call` deadline when it is
/// lost, and a lost DmText is hinted again on the outbox's re-ask cadence.
/// The held route stays in use when nothing newer is published, whether
/// the probe answers or not, and with no record to re-read: a probe whose
/// answer was lost on its way back (a one-way loss) cannot be told from a
/// dead route, and with no newer record there is nothing to use instead,
/// so dropping it would only lose the fast path; the owner's republish is
/// the heal (S6). The round still counts toward unreachable (S4), and the
/// held blob stays failure-cached for 120 s, so a refresh after a send
/// error does not take it back.
async fn suspect_round(host: &dyn FriendHost, node: &TransportNode, to: &str) {
    let live = node.liveness();
    let held = route::cached(node, to);
    if live.begin_round(to, held.clone()) {
        reachability(host, to, false);
    }
    let Some((record, blob, seq)) = route::read_profile_route(host, to).await else {
        tracing::debug!(to = %to, "suspect route: no route record to re-read");
        return;
    };
    let fetched = live.observe(to, seq);
    let same = held.as_deref() == Some(blob.as_slice());
    if fetched == Fetched::Stale {
        tracing::debug!(to = %to, seq, "suspect route: stale record ignored");
        return;
    }
    // Share the dead blob's probe slot with inline profile refreshes
    // (plan RV1g B1/B3, RH1r-c S2/S3).
    match live.route_read_on(&record, &blob, u64::from(seq)) {
        DeadRouteVerdict::Held => return,
        DeadRouteVerdict::Probe => {
            probe_beside(host, to, blob);
            return;
        }
        DeadRouteVerdict::NotDead | DeadRouteVerdict::Cleared => {}
    }
    if fetched == Fetched::Newer && !same {
        tracing::info!(to = %to, seq, "suspect route: the peer republished; using it");
        route::cache(node, to, blob);
    } else {
        probe_beside(host, to, blob);
    }
}

/// Run [`probe`] in the unlock scope, off the send's path.
fn probe_beside(host: &dyn FriendHost, to: &str, blob: Vec<u8>) {
    let owned = host.owned();
    let to = to.to_string();
    host.scope()
        .spawn_or_drop("suspect route probe", async move {
            probe(owned.as_ref(), &to, blob).await;
        });
}

/// Probe the peer's route once with an `app_call` (S2): any answer, a NAK
/// included, shows it delivers and clears its failure cache; with no
/// answer it stays failure-cached, and in use ([`suspect_round`]).
async fn probe(host: &dyn FriendHost, to: &str, blob: Vec<u8>) {
    let Ok(node) = host.node() else {
        return;
    };
    // The probe is the suspect round's one re-test, at most once per round
    // (S2, S3).
    match node.probe_route(&blob).await {
        Probed::Answered => {
            tracing::info!(to = %to, "suspect route answered its probe");
            route::cache(&node, to, blob);
            if node.liveness().probe_answered(to) {
                reachability(host, to, true);
            }
        }
        Probed::ImportFailed(error) => {
            tracing::debug!(to = %to, %error, "suspect route probe: import failed");
            route::invalidate_blob(&node, to, &blob);
        }
        Probed::Silent(error) => {
            tracing::info!(to = %to, %error, "suspect route did not answer its probe");
        }
    }
}
