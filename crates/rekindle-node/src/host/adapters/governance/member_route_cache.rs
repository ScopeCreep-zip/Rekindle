//! The Host's member routes (plan E3.4 S4, "Routes"), on both modes: each
//! member's verified `M`/6 route in its community's
//! [`MemberRouteCache`](rekindle_presence::community::MemberRouteCache),
//! fed by a ring change or a `HeadAdvanced` hint naming subkey 6
//! (`watch`, `head_advanced`) and by a pinned cold read on first use
//! (`rekindle_governance_runtime::member_routes::read_member_route_at`,
//! `RoutePin::Joined`). Every Host call to a member goes through
//! [`HostGovernance::call_member_route`]: a `RouteUnusable` forgets the
//! route and re-reads `M`/6 exactly once; only a changed blob is called
//! again, and an unchanged one is a visible `Unreachable`, never a retry
//! loop (the outbox row's own retry decides any later attempt).

use rekindle_codec::member_record::{routes::decode_member_routes, RecordScope};
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::member_routes::{read_member_route_at, RoutePin};
use rekindle_governance_runtime::GovernanceRuntimeDeps as _;
use rekindle_records::lease::LeaseId;
use rekindle_types::dht_layout::member_record::ROUTES;
use rekindle_types::id::PseudonymKey;

use super::lifecycle::RouteCallError;
use super::HostGovernance;

/// Why a call to a member did not land.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberCallError {
    /// The member is not in the roster, or its record holds no route.
    #[error("the member has no route")]
    NoRoute,
    /// The route was unusable and `M`/6 held no newer one.
    #[error("the member is unreachable: its route is unusable and no newer one is published")]
    Unreachable,
    /// The call itself failed.
    #[error("the call failed: {0}")]
    Call(String),
}

impl HostGovernance {
    /// Take `author`'s verified route blob at DHT `seq` into `community`'s
    /// cache.
    fn observe_route(&self, community: &str, author: &PseudonymKey, seq: u64, blob: Vec<u8>) {
        let mut map = self.ctx.communities.map().write();
        if let Some(c) = map.get_mut(community) {
            c.member_routes.observe(author, seq, blob);
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
        self.observe_route(community, &author, seq, routes.general);
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
    /// names, pinned to that op's owner, into the cache; the blob read.
    async fn read_route_cold(&self, community: &str, member: &PseudonymKey) -> Option<Vec<u8>> {
        let (record, owner) = {
            let state = self.ctx.communities.governance_state(community)?;
            let m = state.members.get(member)?;
            (m.member_record.clone(), m.member_record_owner.clone())
        };
        let port = self.record_port().ok()?;
        let pin = RoutePin::Joined(&owner);
        match read_member_route_at(&*port, community, &record, member, pin).await {
            Ok(Some((seq, blob))) => {
                self.observe_route(community, member, seq, blob.clone());
                Some(blob).filter(|b| !b.is_empty())
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
            Some(blob) => Some(blob),
            None => self.read_route_cold(community, member).await,
        }
    }

    /// `app_call` `member` in `community` with `payload` (unframed,
    /// sealed) at its route ([`Self::member_route`]). On `RouteUnusable`
    /// the route is forgotten and `M`/6 re-read once; the call is retried
    /// once only when that read holds a different blob.
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
            Err(RouteCallError::Failed(e)) => return Err(MemberCallError::Call(e)),
            Err(RouteCallError::Unusable) => {}
        }
        self.forget_route(community, member);
        let fresh = self.read_route_cold(community, member).await;
        let Some(fresh) = fresh.filter(|b| *b != route) else {
            self.forget_route(community, member);
            tracing::warn!(
                community,
                "a member is unreachable: its route is unusable and M/6 holds no newer one"
            );
            return Err(MemberCallError::Unreachable);
        };
        match self.call_route_typed(&fresh, payload).await {
            Ok(reply) => Ok(reply),
            Err(RouteCallError::Failed(e)) => Err(MemberCallError::Call(e)),
            Err(RouteCallError::Unusable) => {
                self.forget_route(community, member);
                tracing::warn!(
                    community,
                    "a member is unreachable: its new route is unusable too"
                );
                Err(MemberCallError::Unreachable)
            }
        }
    }

    /// Forget `member`'s cached route (the transport said it is unusable).
    fn forget_route(&self, community: &str, member: &PseudonymKey) {
        let mut map = self.ctx.communities.map().write();
        if let Some(c) = map.get_mut(community) {
            c.member_routes.forget(member);
        }
    }

    /// The members of `community`'s roster other than us (joined, not
    /// departed, not banned).
    fn roster_others(&self, community: &str) -> Vec<PseudonymKey> {
        let me = self
            .identity_secret_impl()
            .map(|secret| derive_community_pseudonym(&secret, community))
            .map(|key| PseudonymKey(key.verifying_key().to_bytes()));
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
        let roster = self.roster_others(community);
        let map = self.ctx.communities.map().read();
        map.get(community)
            .map(|c| c.member_routes.routes_for(&roster))
            .unwrap_or_default()
            .into_iter()
            .map(|(p, blob)| (p.to_hex(), blob))
            .collect()
    }

    /// [`Self::routed_members`] whose beat shows them present now (plan
    /// E3.4 S5's mesh peers: roster ∩ `M`/6 ∩ beat live): the members a
    /// voice session or a file fetch can reach.
    #[must_use]
    pub fn reachable_members(&self, community: &str) -> Vec<(String, Vec<u8>)> {
        let mut members = self.routed_members(community);
        members.retain(|(p, _)| self.member_beat_shown(community, p).is_present());
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
}
