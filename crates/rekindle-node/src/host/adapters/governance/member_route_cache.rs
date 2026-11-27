//! The Host's member routes (plan E3.4 S4, "Routes"), on both modes: each
//! member's verified `M`/6 route in its community's
//! [`MemberRouteCache`](rekindle_presence::community::MemberRouteCache),
//! fed by a ring change or a `HeadAdvanced` hint naming subkey 6
//! (`watch`, `head_advanced`) and by a pinned cold read on first use
//! (`rekindle_governance_runtime::member_routes::read_member_route_at`,
//! `RoutePin::Joined`). Every Host call to a member goes through
//! [`HostGovernance::call_member_route`]: a `RouteUnusable` or a call
//! timeout (Veilid 0.5.8 loses a call into a released route as a timeout)
//! forgets the route and re-reads `M`/6 exactly once; only a changed blob
//! is called again, and an unchanged one is a visible `Unreachable`, never
//! a retry loop (the outbox row's own retry decides any later attempt).

use rekindle_codec::member_record::{routes::decode_member_routes, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::member_routes::{read_member_route_at, RoutePin};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::lease::LeaseId;
use rekindle_route::dead_routes::Observed;
use rekindle_types::dht_layout::member_record::ROUTES;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

/// Why a call to a member did not land.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberCallError {
    /// The member is not in the roster, or its record holds no route.
    #[error("the member has no route")]
    NoRoute,
    /// The route was unusable or the call to it was lost, and `M`/6 held
    /// no newer one (or the newer one failed too).
    #[error("the member is unreachable: its route failed and no newer one is published")]
    Unreachable,
    /// The call itself failed.
    #[error("the call failed: {0}")]
    Call(String),
}

impl HostGovernance {
    /// Take `author`'s verified route blob at DHT `seq` into `community`'s
    /// cache, unless the transport still knows it dead after this read
    /// (plan RV1g B3: only a strictly newer seq revives a dead blob); the
    /// read's verdict, a probe due included ([`Observed::Probe`]).
    pub(super) fn observe_route(
        &self,
        community: &str,
        author: &PseudonymKey,
        seq: u64,
        blob: Vec<u8>,
    ) -> Observed {
        let seen = self.route_read_at(&blob, seq);
        let dead = self.route_declared_dead(&blob);
        self.take_route(community, author, seq, blob, seen, dead);
        seen
    }

    /// [`Self::observe_route`] off a caller that does not wait on a probe:
    /// one it asks for runs on the community's scope.
    fn observe_route_or_probe(
        &self,
        community: &str,
        author: &PseudonymKey,
        seq: u64,
        blob: Vec<u8>,
    ) {
        if self.observe_route(community, author, seq, blob.clone()) == Observed::Probe {
            self.spawn_member_route_probe(community, author, seq, blob);
        }
    }

    /// Read `record`'s routes (`M`/6) through `lease`, forced through the
    /// hinted read (a ValueChange carries only its first subkey's value,
    /// veilid-core 0.5.8 `veilid_state.rs:334-336`), verify them under the
    /// record's member and cache them; the DHT seq read.
    pub(super) async fn read_member_routes(
        &self,
        community: &str,
        record: &str,
        lease: LeaseId,
    ) -> Option<u64> {
        let author = self.author_of_record(community, record)?;
        let (seq, bytes) = self.read_hinted_value(lease, ROUTES).await.ok().flatten()?;
        let at = RecordScope {
            community_id: community,
            member_record: record,
        };
        let routes = decode_member_routes(&bytes, at, &author.0).ok()?;
        self.observe_route_or_probe(community, &author, seq, routes.general);
        Some(seq)
    }

    /// A ring change naming the routes (subkey 6): read them into the cache.
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

    /// Read `member`'s routes cold from the record its merged `MemberJoined`
    /// names, pinned to that op's owner, into the cache; the blob read,
    /// unless it is empty or the transport knows it dead (plan RV1g B1).
    pub(crate) async fn read_route_cold(
        &self,
        community: &str,
        member: &PseudonymKey,
    ) -> Option<Vec<u8>> {
        let (_, blob) = self.read_route_pinned(community, member).await?;
        Some(blob).filter(|b| !b.is_empty() && !self.route_declared_dead(b))
    }

    /// [`Self::read_route_cold`]'s pinned read: the seq and blob as read.
    async fn read_route_pinned(
        &self,
        community: &str,
        member: &PseudonymKey,
    ) -> Option<(u64, Vec<u8>)> {
        let (record, owner) = {
            let state = self.ctx.communities.governance_state(community)?;
            let m = state.members.get(member)?;
            (m.member_record.clone(), m.member_record_owner.clone())
        };
        let port = self.record_port().ok()?;
        let pin = RoutePin::Joined(&owner);
        match read_member_route_at(&*port, community, &record, member, pin).await {
            Ok(Some((seq, blob))) => {
                self.observe_route_or_probe(community, member, seq, blob.clone());
                Some((seq, blob))
            }
            Ok(None) => None,
            Err(error) => {
                tracing::debug!(community, %error, "a member's route was not read");
                None
            }
        }
    }

    /// `member`'s route in `community`: the cached one, else a pinned read
    /// of its `M`/6. `None` for a non-member or a record with no routes.
    pub(crate) async fn member_route(
        &self,
        community: &str,
        member: &PseudonymKey,
    ) -> Option<Vec<u8>> {
        if !self.is_key_reader(community, member) {
            return None;
        }
        let cached = {
            let map = self.ctx.communities.map().read();
            map.get(community)
                .and_then(|c| c.member_routes.get(member).map(<[u8]>::to_vec))
        };
        match cached {
            Some(blob) if !self.route_declared_dead(&blob) => Some(blob),
            Some(_) => {
                self.forget_dead_routes(community);
                self.read_route_cold(community, member).await
            }
            None => self.read_route_cold(community, member).await,
        }
    }

    /// `app_call` `member` in `community` with `payload` (unframed,
    /// sealed) at its route ([`Self::member_route`]). On `RouteUnusable`
    /// or a timeout ([`super::lifecycle::RouteCallError::rereads_route`]) the route is
    /// forgotten and `M`/6 re-read once; the call is retried once only
    /// when that read holds a different blob.
    ///
    /// # Errors
    /// [`MemberCallError`].
    pub(crate) async fn call_member_route(
        &self,
        community: &str,
        member: &PseudonymKey,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        let route = self
            .member_route(community, member)
            .await
            .ok_or(MemberCallError::NoRoute)?;
        match self.call_route_typed(&route, payload.clone()).await {
            Ok(reply) => return Ok(reply),
            Err(e) if e.rereads_route() => {}
            Err(e) => return Err(MemberCallError::Call(e.to_string())),
        }
        let fresh = self.reread_route(community, member).await;
        let Some(fresh) = fresh.filter(|b| *b != route) else {
            self.forget_route(community, member);
            tracing::warn!(
                community,
                "a member is unreachable: its route failed and M/6 holds no newer one"
            );
            return Err(MemberCallError::Unreachable);
        };
        match self.call_route_typed(&fresh, payload).await {
            Ok(reply) => Ok(reply),
            Err(e) if e.rereads_route() => {
                self.forget_route(community, member);
                tracing::warn!(
                    community,
                    "a member is unreachable: its new route failed too"
                );
                Err(MemberCallError::Unreachable)
            }
            Err(e) => Err(MemberCallError::Call(e.to_string())),
        }
    }

    /// A send to `member`'s cached route failed: forget it and read its
    /// `M`/6 once, pinned (the one re-read a dropped hint costs). The new
    /// route, or `None` when the read holds no usable one (nothing, the
    /// blob that failed, or one the transport knows dead); then D8's cap
    /// (`rekindle_presence::community::reread_cap`) holds the next re-read
    /// off the network until its window passes or a newer seq arrives.
    async fn reread_route(&self, community: &str, member: &PseudonymKey) -> Option<Vec<u8>> {
        let now = tokio::time::Instant::now();
        let may = {
            let map = self.ctx.communities.map().read();
            map.get(community)
                .is_some_and(|c| c.member_routes.may_reread(member, now))
        };
        if !may {
            self.forget_route(community, member);
            tracing::debug!(community, "a member's M/6 re-read is capped (D8)");
            return None;
        }
        let failed = self.forget_route(community, member);
        let read = self.read_route_pinned(community, member).await;
        let mut map = self.ctx.communities.map().write();
        let c = map.get_mut(community)?;
        let dead = |blob: &[u8]| self.route_declared_dead(blob);
        c.member_routes
            .reread(member, failed.as_deref(), read, dead, now)
    }

    /// [`Self::reread_route`] by pseudonym hex, for the gossip mesh's
    /// re-resolve after a failed send (both modes' `GossipDeps`): `None`
    /// for a non-member or a record with no routes.
    pub async fn reread_member_route(&self, community: &str, member_hex: &str) -> Option<Vec<u8>> {
        let member = super::moderation::pseudonym(member_hex)?;
        if !self.is_key_reader(community, &member) {
            return None;
        }
        self.reread_route(community, &member).await
    }

    /// Read `M`/6 once, pinned, for every roster member of `community`
    /// but us whose route is not cached yet (plan E3.4 S4: the rotator
    /// fills its cache before a key is distributed, so a member outside
    /// the watch ring is not left out by a cold cache).
    pub async fn fill_member_routes(&self, community: &str) {
        let cold: Vec<PseudonymKey> = {
            let roster = self.roster_others(community);
            let map = self.ctx.communities.map().read();
            let Some(c) = map.get(community) else {
                return;
            };
            roster
                .into_iter()
                .filter(|p| c.member_routes.get(p).is_none())
                .collect()
        };
        let reads = cold.iter().map(|p| self.read_route_cold(community, p));
        futures::future::join_all(reads).await;
    }

    /// Forget `member`'s cached route (the transport said it is unusable);
    /// the blob that was held.
    fn forget_route(&self, community: &str, member: &PseudonymKey) -> Option<Vec<u8>> {
        let mut map = self.ctx.communities.map().write();
        map.get_mut(community)
            .and_then(|c| c.member_routes.forget(member))
    }

    /// Forget every cached route of `community` the transport knows dead
    /// (Veilid's `RouteChange`, or timed-out calls; plan RV1g B3).
    fn forget_dead_routes(&self, community: &str) {
        let mut map = self.ctx.communities.map().write();
        if let Some(c) = map.get_mut(community) {
            c.member_routes
                .forget_blobs(|blob| self.route_declared_dead(blob));
        }
    }

    /// Our pseudonym in `community`, derived from the identity.
    pub(crate) fn me_in(&self, community: &str) -> Option<PseudonymKey> {
        self.identity_secret_impl()
            .map(|secret| derive_community_pseudonym(&secret, community))
            .map(|key| PseudonymKey(key.verifying_key().to_bytes()))
    }

    /// The members of `community`'s roster other than us (joined, not
    /// departed, not banned).
    fn roster_others(&self, community: &str) -> Vec<PseudonymKey> {
        let me = self.me_in(community);
        let Some(state) = self.ctx.communities.governance_state(community) else {
            return Vec::new();
        };
        (state.members.keys())
            .filter(|p| !state.is_departed(p) && !state.bans.contains(*p))
            .filter(|p| me.as_ref() != Some(*p))
            .cloned()
            .collect()
    }

    /// Every roster member of `community` but us with a cached route
    /// (pseudonym hex, blob): the recipients of a key rotation (plan E3.4
    /// S4: every member with a route, not only those seen online).
    #[must_use]
    pub fn routed_members(&self, community: &str) -> Vec<(String, Vec<u8>)> {
        self.forget_dead_routes(community);
        let roster = self.roster_others(community);
        let map = self.ctx.communities.map().read();
        map.get(community)
            .map(|c| c.member_routes.routes_for(&roster))
            .unwrap_or_default()
            .into_iter()
            .map(|(p, blob)| (p.to_hex(), blob))
            .collect()
    }

    /// [`Self::routed_members`] who may be present now (plan E3.4 S5's mesh
    /// peers: roster ∩ `M`/6 ∩ not shown offline): a member whose beat shows
    /// them present, or one whose beat has not reached us yet. A member only
    /// an expired, `Offline` or `Departed` beat shows is left out; an unknown
    /// one is not offline (a joiner's first beat can arrive after a post it
    /// must hear). The members a voice session or a file fetch can reach.
    #[must_use]
    pub fn reachable_members(&self, community: &str) -> Vec<(String, Vec<u8>)> {
        let mut members = self.routed_members(community);
        members.retain(|(p, _)| {
            crate::host::adapters::presence::view::may_be_present(&self.ctx, community, p)
        });
        members
    }

    /// `member_hex`'s cached route in `community`, for a caller that cannot
    /// wait for a read; `None` when not cached or not a member.
    #[must_use]
    pub fn cached_member_route(&self, community: &str, member_hex: &str) -> Option<Vec<u8>> {
        let member = super::moderation::pseudonym(member_hex)?;
        if !self.is_key_reader(community, &member) {
            return None;
        }
        let map = self.ctx.communities.map().read();
        map.get(community)
            .and_then(|c| c.member_routes.get(&member).map(<[u8]>::to_vec))
    }

    /// [`Self::call_member_route`] by pseudonym hex.
    ///
    /// # Errors
    /// [`MemberCallError`]; `NoRoute` for a malformed pseudonym.
    pub(crate) async fn call_member_hex(
        &self,
        community: &str,
        member_hex: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        let member = super::moderation::pseudonym(member_hex).ok_or(MemberCallError::NoRoute)?;
        self.call_member_route(community, &member, payload).await
    }

    /// The netsim's entry to one member call (plan E3.4 S4 netsim B):
    /// [`Self::call_member_hex`] as it is, built only with the
    /// `netsim-entry` feature, which only this crate's own dev-dependency
    /// turns on (production calls a member only through the MEK answer,
    /// the key transfer, files and voice).
    ///
    /// # Errors
    /// [`MemberCallError`].
    #[cfg(feature = "netsim-entry")]
    pub async fn netsim_call_member(
        &self,
        community: &str,
        member_hex: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        self.call_member_hex(community, member_hex, payload).await
    }
}

#[cfg(test)]
#[path = "member_route_cache_tests.rs"]
mod tests;
