//! Our own op record's watch (ADR 0022 rule 4, plan E3.2 S7d-keys.9), on
//! both modes: each device watches its own `M(p,d)`/2 in every community it
//! holds, since the ring watches only successors (`watch_ring`) and a copy
//! of this data root writes there under our key before any of its ops
//! reach a peer. It is armed with the ring's watches, on every ring move
//! (one more watcher per record: ADR 0011's bound is ≤ 5), and the record
//! pool re-arms it on `count == 0` as it does every held lease's watch (D4,
//! the one retry layer; 0.5.8 `storage_manager/watch_value.rs:2080`
//! `inbound_value_changed` delivers each change of a watched subkey). A
//! change runs `own_record_watch::own_record_changed`, whose retirement
//! check stops signing at once and is told to the user once a session.

use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_governance_runtime::{GovernanceRuntimeDeps, GovernanceRuntimeError};
use rekindle_records::port::OpenIntent;
use rekindle_transport::broadcast::dht_writes::{self, LeaseId};
use rekindle_types::dht_layout::member_record::GOV_HEAD;

use super::HostGovernance;
use crate::host::community::state::CommunityState;

#[cfg(test)]
#[path = "own_record_watch_tests.rs"]
mod tests;

/// Is this the first retirement notice of `community` this session? A
/// retired key stays retired, so the user is told once.
pub(super) fn first_retired_notice(community: &mut CommunityState) -> bool {
    !std::mem::replace(&mut community.own_key_retired_shown, true)
}

impl HostGovernance {
    /// The community whose own-record watch holds `record`.
    pub(super) fn community_of_own_record(&self, record: &str) -> Option<String> {
        let map = self.ctx.communities.map().read();
        let mut ids: Vec<&String> = map
            .values()
            .filter(|c| {
                c.own_record_watch
                    .as_ref()
                    .is_some_and(|(r, _)| r == record)
            })
            .map(|c| &c.id)
            .collect();
        ids.sort_unstable();
        ids.first().map(|id| (*id).clone())
    }

    /// Watch our op record's head (`M(p,d)`/2) in `community_id`, moving
    /// the watch when the merged state binds another record (a re-join or
    /// a re-enrolment): called by the ring move, under its lock.
    pub(super) async fn arm_own_record_watch_locked(
        &self,
        community_id: &str,
        node: &rekindle_transport::TransportNode,
    ) {
        let state = self.governance_state_impl(community_id).unwrap_or_default();
        let wanted =
            rekindle_governance_runtime::device_signer::own_op_record(self, community_id, &state)
                .ok();
        let current = {
            let map = self.ctx.communities.map().read();
            map.get(community_id)
                .and_then(|c| c.own_record_watch.as_ref().map(|(r, _)| r.clone()))
        };
        if wanted == current {
            return;
        }
        let gone = (self.ctx.communities.map().write().get_mut(community_id))
            .and_then(|c| c.own_record_watch.take().map(|(_, lease)| lease));
        if let Some(lease) = gone {
            dht_writes::release(node, lease).await;
        }
        let Some(record) = wanted else {
            return;
        };
        let lease = match dht_writes::acquire_str(node, &record, None, OpenIntent::Check).await {
            Ok(lease) => lease,
            Err(e) => {
                tracing::debug!(community_id, record, error = %e, "own record did not open");
                return;
            }
        };
        // A refused watch stays set in the pool, which re-arms it (D4).
        if let Err(e) = dht_writes::watch_leased(node, lease, &[GOV_HEAD]).await {
            tracing::debug!(community_id, record, error = %e, "own record watch not yet set");
        }
        let surplus: Option<LeaseId> = {
            let mut map = self.ctx.communities.map().write();
            match map.get_mut(community_id) {
                Some(c) if c.own_record_watch.is_none() => {
                    c.own_record_watch = Some((record, lease));
                    None
                }
                _ => Some(lease),
            }
        };
        if let Some(lease) = surplus {
            dht_writes::release(node, lease).await;
        }
    }

    /// Our op record changed: on a head change (or one we did not observe)
    /// receive what a copy wrote and check our key; a retirement stops
    /// signing (`OpSigner::checked` refuses from the merged state) and is
    /// told to the user once.
    pub(super) async fn on_own_record_change(
        &self,
        community_id: &str,
        record: &str,
        subkeys: &[u32],
    ) {
        if (!subkeys.is_empty() && !subkeys.contains(&GOV_HEAD))
            || self.ctx.communities.own_removal(community_id).is_some()
        {
            return;
        }
        match rekindle_governance_runtime::own_record_watch::own_record_changed(
            self,
            community_id,
            record,
        )
        .await
        {
            Ok(()) => {}
            Err(error @ GovernanceRuntimeError::OwnKeyRetired { .. }) => {
                let first = (self.ctx.communities.map().write().get_mut(community_id))
                    .is_some_and(first_retired_notice);
                tracing::warn!(community_id, %error, first, "own device key retired");
                if first {
                    GovernanceRuntimeDeps::emit_event(
                        self,
                        GovernanceRuntimeEvent::OwnKeyRetired {
                            community_id: community_id.to_owned(),
                            notice: error.to_string(),
                        },
                    );
                }
            }
            Err(error) => {
                tracing::debug!(community_id, record, %error, "own record change not read");
            }
        }
    }
}
