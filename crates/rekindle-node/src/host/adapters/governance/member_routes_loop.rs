//! The writer of our routes in every community (plan E3.2 S7c.8 item (4),
//! the E3.4 `MemberRoutes` writer), one task per unlock on both modes: it
//! follows our general route (`RoutePort::own_state`) and, for each
//! `Available` blob, rewrites `M`/6 of every membership whose last write
//! was another blob. A route Veilid reports dead is replaced by
//! `OwnRoutes` (transport `broadcast/node/routes.rs`), so an invite's
//! joiner, who reads the inviter's route from `M`/6 at join time
//! (`rekindle_governance_runtime::member_routes`), reaches the inviter's
//! current route, as a Matrix invite's server is resolved at use time.
//! A membership created later writes its own first `M`/6 with its record
//! (`member_card::publish_own_record`).

use std::collections::BTreeMap;

use rekindle_governance_runtime::member_card::OwnRecord;
use rekindle_governance_runtime::member_routes::write_routes;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_records::port::{RouteClass, RouteState};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let routes = match host.transport() {
        Ok(transport) => transport.routes(),
        Err(error) => {
            tracing::warn!(%error, "member routes: no transport, routes not published");
            return;
        }
    };
    let mut general = routes.own_state(RouteClass::General);
    // The blob last written to each community's `M`/6 this unlock.
    let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    loop {
        let state = general.borrow_and_update().clone();
        if let RouteState::Available { blob } = state {
            for community_id in host.sync_targets() {
                if stop.is_cancelled() {
                    return;
                }
                if written.get(&community_id) == Some(&blob) {
                    continue;
                }
                match host.publish_own_routes(&community_id, &blob).await {
                    Ok(()) => {
                        written.insert(community_id, blob.clone());
                    }
                    Err(error) => {
                        tracing::warn!(community = %community_id, %error, "member routes not written");
                    }
                }
            }
        }
        match stop.run_until_cancelled(general.changed()).await {
            Some(Ok(())) => {}
            Some(Err(_)) | None => return,
        }
    }
}

impl HostGovernance {
    /// Our member record in `community_id`, as our `MemberJoined` names it.
    pub(super) fn own_member_record(&self, community_id: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let c = map.get(community_id)?;
        let me = c.my_pseudonym_key.as_deref()?;
        let state = c.governance_state.as_ref()?;
        state
            .members
            .iter()
            .find(|(p, _)| hex::encode(p.0) == me)
            .map(|(_, m)| m.member_record.clone())
    }

    /// Write `blob` to our `M`/6 in `community_id`.
    ///
    /// # Errors
    /// We hold no member record or keys there, or the write failed.
    async fn publish_own_routes(&self, community_id: &str, blob: &[u8]) -> Result<(), String> {
        let record = self
            .own_member_record(community_id)
            .ok_or("no member record of ours")?;
        let writer = self.member_record_writer(community_id)?;
        let secret = self
            .identity_secret_impl()
            .ok_or("no identity is unlocked")?;
        let pseudonym =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
        let port = self.record_port().map_err(|e| e.to_string())?;
        let own = OwnRecord {
            community_id,
            record: &record,
            writer: &writer,
            pseudonym: &pseudonym,
        };
        write_routes(&*port, &own, blob, self.ctx.clock.now_ms()).await
    }
}
