//! The Host's owner-pinned M/6 reader and S4 member-call recovery.

use std::sync::Arc;

use rekindle_codec::member_record::{routes::decode_member_routes, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::member_card::member_device_records;
use rekindle_governance_runtime::member_routes::{read_member_route_at, RoutePin};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_presence::community::RouteVersion;
use rekindle_records::lease::LeaseId;
use rekindle_route::dead_routes::DeadRouteVerdict;
use rekindle_transport::route_trace::blob_tag;
use rekindle_types::dht_layout::member_record::ROUTES;
use rekindle_types::id::PseudonymKey;
use rekindle_utils::log_scrub::tag;
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::{member_calls::FailedRoute, HostGovernance};

/// A device record and the descriptor owner pinned by its endorsement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RouteRecord {
    pub record: String,
    pub owner: String,
}

pub(super) fn trace_route(
    control_type: &str,
    record: &str,
    version: Option<&RouteVersion>,
    read_disposition: &str,
) {
    tracing::debug!(
        control_type,
        record_tag = %tag(record),
        route_tag = version.map(|r| blob_tag(&r.blob)),
        seq = version.map(|r| r.seq),
        read_disposition,
        "member route resolution"
    );
}

impl HostGovernance {
    pub(super) fn route_records(&self, community: &str, author: &PseudonymKey) -> Vec<RouteRecord> {
        let map = self.ctx.communities.map().read();
        let Some(state) = map.get(community).and_then(|c| c.governance_state.as_ref()) else {
            return Vec::new();
        };
        member_device_records(state, community, author)
            .into_iter()
            .map(|at| RouteRecord {
                record: at.record.to_owned(),
                owner: at.owner.to_owned(),
            })
            .collect()
    }

    pub(super) fn binding_of_record(
        &self,
        community: &str,
        record: &str,
    ) -> Option<(PseudonymKey, RouteRecord)> {
        let author = self.author_of_record(community, record)?;
        let binding = self
            .route_records(community, &author)
            .into_iter()
            .find(|at| at.record == record)?;
        Some((author, binding))
    }

    pub(super) fn cached_route(&self, community: &str, record: &str) -> Option<RouteVersion> {
        let map = self.ctx.communities.map().read();
        map.get(community)?.member_routes.get(record).cloned()
    }

    pub(super) fn route_read_lock(&self, community: &str, record: &str) -> Option<Arc<Mutex<()>>> {
        let mut map = self.ctx.communities.map().write();
        Some(map.get_mut(community)?.member_routes.read_lock(record))
    }

    pub(super) fn may_read_route(&self, community: &str, record: &str, now: Instant) -> bool {
        let map = self.ctx.communities.map().read();
        map.get(community)
            .is_some_and(|c| c.member_routes.may_reread(record, now))
    }

    /// Install a verified version before consulting transport liveness.
    /// Obsolete reads have no effect on either liveness or the cache.
    fn observe_route(
        &self,
        community: &str,
        record: &str,
        version: &RouteVersion,
        failed_blob: Option<&[u8]>,
        control: &str,
    ) -> Option<DeadRouteVerdict> {
        let (seen, disposition) = {
            let mut map = self.ctx.communities.map().write();
            let cache = &mut map.get_mut(community)?.member_routes;
            if !cache.verify(record, version) {
                trace_route(control, record, Some(version), "stale");
                return None;
            }
            let seen = self.route_read_at(record, &version.blob, version.seq);
            let dead = self.route_declared_dead(&version.blob);
            let unchanged = Some(version.blob.as_slice()) == failed_blob;
            if !unchanged || dead {
                cache.set_usable(record, version, !dead);
            }
            let disposition = if unchanged {
                "unchanged"
            } else if dead {
                "dead"
            } else {
                "usable"
            };
            (seen, disposition)
        };
        trace_route(control, record, Some(version), disposition);
        if seen == DeadRouteVerdict::Cleared {
            self.drain_pending_mesh(community);
        }
        Some(seen)
    }

    /// Read the watched subkey through its hinted lease, checking the
    /// merged binding's owner before using the value.
    pub(super) async fn read_member_routes(
        &self,
        community: &str,
        record: &str,
        lease: LeaseId,
    ) -> Option<u64> {
        let (author, binding) = self.binding_of_record(community, record)?;
        if self.record_port().ok()?.owner_of(lease).ok()? != binding.owner {
            return None;
        }
        let (seq, bytes) = self.read_hinted_value(lease, ROUTES).await.ok().flatten()?;
        let at = RecordScope {
            community_id: community,
            member_record: record,
        };
        let routes = decode_member_routes(&bytes, at, &author.0).ok()?;
        if self.binding_of_record(community, record)?.1 != binding {
            return None;
        }
        let version = RouteVersion {
            seq,
            blob: routes.general,
        };
        let seen = self.observe_route(community, record, &version, None, "watch");
        if seen == Some(DeadRouteVerdict::Probe) {
            self.spawn_member_route_probe(community, record, version);
        }
        seen.map(|_| seq)
    }

    pub(super) async fn ring_route_moved(&self, community: &str, record: &str) {
        let lease = {
            let map = self.ctx.communities.map().read();
            map.get(community)
                .and_then(|c| c.watch_ring.get(record).copied())
        };
        if let Some(lease) = lease {
            self.read_member_routes(community, record, lease).await;
        }
    }

    /// One pinned read. A read returning a retired binding or an older
    /// version cannot schedule a probe or return that version to a caller.
    pub(super) async fn read_route_pinned(
        &self,
        community: &str,
        author: &PseudonymKey,
        binding: &RouteRecord,
        failed_blob: Option<&[u8]>,
        control: &str,
        wait_for_probe: bool,
    ) -> Option<RouteVersion> {
        let record = &binding.record;
        let port = self.record_port().ok()?;
        let read = read_member_route_at(
            &*port,
            community,
            record,
            author,
            RoutePin::Joined(&binding.owner),
        )
        .await;
        let version = match read {
            Ok(Some((seq, blob))) => RouteVersion { seq, blob },
            Ok(None) => {
                trace_route(control, record, None, "missing");
                return None;
            }
            Err(error) => {
                trace_route(control, record, None, "read_failed");
                tracing::debug!(community, %error, "member route read failed");
                return None;
            }
        };
        if self.binding_of_record(community, record)?.1 != *binding {
            trace_route(control, record, Some(&version), "binding_changed");
            return None;
        }
        let seen = self.observe_route(community, record, &version, failed_blob, control)?;
        if seen == DeadRouteVerdict::Probe {
            if wait_for_probe {
                self.probe_member_route(community, record, &version).await;
            } else {
                self.spawn_member_route_probe(community, record, version.clone());
            }
        }
        self.cached_route(community, record)
            .filter(|held| *held == version)
    }

    /// Read a member's effective routes once, preferring the record that
    /// issued the invite when it is still effective. The historical
    /// invitation record remains the proof locator.
    pub(super) async fn read_inviter_device_route(
        &self,
        community: &str,
        author: &PseudonymKey,
        inviter_record: &str,
    ) -> Option<Vec<u8>> {
        let mut bindings = self.route_records(community, author);
        bindings.sort_by_key(|binding| binding.record != inviter_record);
        for binding in bindings {
            if let Some(version) = self
                .read_route_pinned(community, author, &binding, None, "bootstrap", true)
                .await
            {
                return Some(version.blob);
            }
        }
        None
    }

    /// Read M/6 once for each effective binding until one answers with a
    /// usable route, under D8's per-record cap.
    pub(crate) async fn read_route_cold(
        &self,
        community: &str,
        member: &PseudonymKey,
    ) -> Option<Vec<u8>> {
        for binding in self.route_records(community, member) {
            if let Some(route) = self
                .route_for_record(community, member, &binding, "resolve")
                .await
            {
                return Some(route.blob);
            }
        }
        None
    }

    pub(super) async fn route_for_record(
        &self,
        community: &str,
        member: &PseudonymKey,
        binding: &RouteRecord,
        control: &str,
    ) -> Option<RouteVersion> {
        if let Some(cached) = self.cached_route(community, &binding.record) {
            if !self.route_declared_dead(&cached.blob) {
                trace_route(control, &binding.record, Some(&cached), "cached");
                return Some(cached);
            }
            return self
                .reread_route(
                    community,
                    member,
                    binding,
                    FailedRoute::Version(&cached),
                    control,
                )
                .await;
        }
        let lock = self.route_read_lock(community, &binding.record)?;
        let _reading = lock.lock().await;
        if let Some(cached) = self
            .cached_route(community, &binding.record)
            .filter(|version| !self.route_declared_dead(&version.blob))
        {
            trace_route(control, &binding.record, Some(&cached), "cached");
            return Some(cached);
        }
        if !self.may_read_route(community, &binding.record, Instant::now()) {
            trace_route(control, &binding.record, None, "capped");
            return None;
        }
        self.read_route_pinned(community, member, binding, None, control, false)
            .await
    }

    /// Resolve a member's route for a send.
    pub(crate) async fn member_route(
        &self,
        community: &str,
        member: &PseudonymKey,
    ) -> Option<Vec<u8>> {
        if !self.is_key_reader(community, member) {
            return None;
        }
        self.read_route_cold(community, member).await
    }

    /// Read a route for every roster member who has none cached.
    pub async fn fill_member_routes(&self, community: &str) {
        let cold: Vec<_> = self
            .roster_others(community)
            .into_iter()
            .filter(|member| {
                self.cached_member_route(community, &member.to_hex())
                    .is_none()
            })
            .collect();
        futures::future::join_all(cold.iter().map(|p| self.read_route_cold(community, p))).await;
    }

    /// Our pseudonym in `community`, derived from the identity.
    pub(crate) fn me_in(&self, community: &str) -> Option<PseudonymKey> {
        self.identity_secret_impl()
            .map(|secret| derive_community_pseudonym(&secret, community))
            .map(|key| PseudonymKey(key.verifying_key().to_bytes()))
    }

    fn roster_others(&self, community: &str) -> Vec<PseudonymKey> {
        let me = self.me_in(community);
        let Some(state) = self.ctx.communities.governance_state(community) else {
            return Vec::new();
        };
        state
            .members
            .keys()
            .filter(|p| {
                !state.is_departed(p) && !state.bans.contains(*p) && me.as_ref() != Some(*p)
            })
            .cloned()
            .collect()
    }

    /// Each roster member with an effective cached route.
    #[must_use]
    pub fn routed_members(&self, community: &str) -> Vec<(String, Vec<u8>)> {
        let me = self.me_in(community);
        let mut map = self.ctx.communities.map().write();
        let Some(c) = map.get_mut(community) else {
            return Vec::new();
        };
        let Some(state) = &c.governance_state else {
            return Vec::new();
        };
        let cache = &mut c.member_routes;
        cache.forget_blobs(|blob| self.route_declared_dead(blob));
        state
            .members
            .keys()
            .filter(|member| me.as_ref() != Some(*member))
            .filter_map(|member| {
                let route = member_device_records(state, community, member)
                    .into_iter()
                    .find_map(|binding| cache.get(binding.record))?;
                Some((member.to_hex(), route.blob.clone()))
            })
            .collect()
    }

    /// Routed members whose beats allow them to be present.
    #[must_use]
    pub fn reachable_members(&self, community: &str) -> Vec<(String, Vec<u8>)> {
        let mut members = self.routed_members(community);
        members.retain(|(p, _)| {
            crate::host::adapters::presence::view::may_be_present(&self.ctx, community, p)
        });
        members
    }

    /// A cached route on an effective record.
    #[must_use]
    pub fn cached_member_route(&self, community: &str, member_hex: &str) -> Option<Vec<u8>> {
        let member = super::moderation::pseudonym(member_hex)?;
        let map = self.ctx.communities.map().read();
        let c = map.get(community)?;
        let state = c.governance_state.as_ref()?;
        state.members.get(&member)?;
        member_device_records(state, community, &member)
            .into_iter()
            .find_map(|binding| c.member_routes.get(binding.record))
            .map(|route| route.blob.clone())
    }

    /// Deliver a verified version as a delayed replica read in the simulator.
    #[cfg(feature = "netsim-entry")]
    pub fn netsim_observe_route_value(
        &self,
        community: &str,
        record: &str,
        value: &rekindle_records::port::RecordValue,
    ) -> bool {
        let Some((author, binding)) = self.binding_of_record(community, record) else {
            return false;
        };
        if value.writer != binding.owner {
            return false;
        }
        let at = RecordScope {
            community_id: community,
            member_record: record,
        };
        let Ok(decoded) = decode_member_routes(&value.data, at, &author.0) else {
            return false;
        };
        let version = RouteVersion {
            seq: u64::from(value.seq),
            blob: decoded.general,
        };
        self.observe_route(community, record, &version, None, "netsim_read")
            .is_some()
    }

    /// Deliver a route death on this simulator reader.
    #[cfg(feature = "netsim-entry")]
    pub fn netsim_declare_member_route_dead(&self, community: &str, blob: Vec<u8>) {
        if let Ok(transport) = self.transport() {
            transport.liveness().declare_dead_by_veilid(&[blob]);
        }
        if let Some(c) = self.ctx.communities.map().write().get_mut(community) {
            c.member_routes
                .forget_blobs(|blob| self.route_declared_dead(blob));
        }
    }
}

#[cfg(test)]
#[path = "member_route_cache_tests.rs"]
mod tests;
