//! The caller side of route evidence source 7 (plan RV2;
//! `crate::liveness::report`): a re-read of a peer's profile that returns
//! the blob our Veilid declared dead queues one `RouteReport` per (peer,
//! blob) through the outbox, on the stream the peer already watches from
//! us: our 1:1 DM chain for a friend, our sender-owned group chain for a
//! roster member who is not one (plan E2.6). It never travels on the dead
//! route, it adds no record, and nothing falls back: the sends to the peer
//! stay in the outbox until the route watch brings a new blob.

use rekindle_codec::capnp_codec::direct::{DirectBody, RouteReport};
use rekindle_dm::group::group_id_hex;
use rekindle_types::friend::FriendshipState;

use super::chat::{client_id_text, mint_client_id};
use crate::operations::friend::{state, FriendError, FriendHost};

/// Whether `blob`, just re-read for `peer`, is one our Veilid declared
/// dead: the first such read of each blob queues its report (beside the
/// caller, on the unlock scope).
pub(crate) fn report_if_dead(host: &dyn FriendHost, peer: &str, blob: &[u8]) -> bool {
    let Ok(node) = host.node() else {
        return false;
    };
    let live = node.liveness();
    if !live.route_declared_dead(blob) {
        return false;
    }
    if let Some(hash) = live.report_due(peer, blob) {
        let (owned, peer) = (host.owned(), peer.to_string());
        host.scope()
            .spawn_or_drop("route unreachable report", async move {
                if let Err(e) = enqueue(owned.as_ref(), &peer, hash).await {
                    tracing::warn!(to = %peer, error = %e, "route report not committed to the outbox");
                }
            });
    }
    true
}

/// Commit the report of `peer`'s blob `blob_hash` to the stream they watch
/// from us.
async fn enqueue(
    host: &dyn FriendHost,
    peer: &str,
    blob_hash: [u8; 32],
) -> Result<(), FriendError> {
    let report = RouteReport { blob_hash };
    let client_id = client_id_text(&mint_client_id(host));
    if state::friend(host, peer).map(|f| f.friendship_state) == Some(FriendshipState::Accepted) {
        let payload = super::outbox::payload(&DirectBody::RouteReport(report))?;
        host.outbox()?
            .enqueue_control(peer, client_id, payload)
            .await?;
        tracing::info!(to = %peer, stream = "dm", "reported the peer's General route unreachable");
        return Ok(());
    }
    let groups = match host.group_dm()?.groups().await {
        Ok(groups) => groups,
        Err(e) => {
            tracing::debug!(to = %peer, error = %e, "groups not read; route report not queued");
            return Ok(());
        }
    };
    // The least group id we share, never the list's order.
    let Some(group) = groups
        .iter()
        .filter(|g| g.roster.member(peer).is_some())
        .map(|g| group_id_hex(&g.group_id))
        .min()
    else {
        tracing::debug!(to = %peer, "neither a friend nor on a roster we hold; no route report");
        return Ok(());
    };
    let payload = super::outbox::payload(&DirectBody::GroupRouteReport(report))?;
    host.outbox()?
        .enqueue_group_control(&group, client_id, payload)
        .await?;
    tracing::info!(to = %peer, stream = "group_dm", "reported the peer's General route unreachable");
    Ok(())
}

/// The stream a report arrived on, as its ingest knows it (design §4,
/// owner 2): our view of the sender's friendship for a 1:1 `RouteReport`,
/// the roster of the group whose chain carried a `GroupRouteReport`.
pub(crate) enum ReportStream<'a> {
    Dm(Option<FriendshipState>),
    Group(&'a rekindle_dm::group::Roster),
}

/// Whether `reporter`'s report counts as evidence against our General
/// route (plan RV2, evidence source 7): only from an accepted friend on
/// its DM chain, or from a writer on the roster of the group whose chain
/// carried it. Anyone else could only make us churn routes, so theirs is
/// dropped at ingest.
pub(crate) fn admitted(reporter: &str, carried: &ReportStream<'_>) -> bool {
    match carried {
        ReportStream::Dm(friendship) => *friendship == Some(FriendshipState::Accepted),
        ReportStream::Group(roster) => roster.member(reporter).is_some(),
    }
}

/// `reporter`'s report of the blob `blob_hash`, read on `carried`: handed
/// to our liveness when [`admitted`]. Whether it was.
pub(crate) fn ingest(
    host: &dyn FriendHost,
    reporter: &str,
    carried: &ReportStream<'_>,
    blob_hash: [u8; 32],
) -> bool {
    if !admitted(reporter, carried) {
        tracing::info!(from = %reporter, "route report from neither a friend nor a roster member; dropped");
        return false;
    }
    if let Ok(node) = host.node() {
        node.liveness().route_reported(reporter, blob_hash);
    }
    true
}

/// A `RouteReport` (the sender's DM chain, or a direct copy, both opened
/// under our ratchet): evidence against our General route from an
/// accepted friend ([`ingest`]). Handled either way: a dropped report is
/// not retried.
pub(crate) fn on_report(host: &dyn FriendHost, sender: &str, report: &RouteReport) -> bool {
    let friendship = state::friend(host, sender).map(|f| f.friendship_state);
    ingest(
        host,
        sender,
        &ReportStream::Dm(friendship),
        report.blob_hash,
    );
    true
}

#[cfg(test)]
mod tests {
    use rekindle_types::friend::FriendshipState;
    use rekindle_types::group_dm::GroupDmRosterMember;

    use super::{admitted, ReportStream};

    fn roster() -> rekindle_dm::group::Roster {
        let members = ['a', 'b', 'c']
            .into_iter()
            .map(|n| GroupDmRosterMember {
                identity: n.to_string().repeat(64),
                profile_record_key: "VLD0:k".into(),
                display_name: n.to_string(),
            })
            .collect();
        rekindle_dm::group::Roster::new(members).expect("a roster of three")
    }

    /// Plan RV2, evidence source 7, design §4 owner 2: a report from a
    /// stranger, a pending or departing friend, or a writer off the roster
    /// is dropped at ingest; only an accepted friend's or a roster
    /// member's counts.
    #[test]
    fn a_report_from_a_non_friend_or_off_the_roster_is_dropped() {
        for friendship in [
            None,
            Some(FriendshipState::PendingOut),
            Some(FriendshipState::Removing),
        ] {
            assert!(
                !admitted("x", &ReportStream::Dm(friendship)),
                "{friendship:?}"
            );
        }
        assert!(admitted(
            "x",
            &ReportStream::Dm(Some(FriendshipState::Accepted))
        ));
        let roster = roster();
        assert!(!admitted(&"d".repeat(64), &ReportStream::Group(&roster)));
        assert!(admitted(&"b".repeat(64), &ReportStream::Group(&roster)));
    }
}
