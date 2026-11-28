//! S4 calls and recovery across a member's effective device records.

use rekindle_codec::capnp_envelope::try_decode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_presence::community::RouteVersion;
use rekindle_types::id::PseudonymKey;
use tokio::time::Instant;

use super::member_route_cache::{trace_route, RouteRecord};
use super::HostGovernance;

/// Why a call to a member did not land.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberCallError {
    /// The member is not in the roster, or its records hold no routes.
    #[error("the member has no route")]
    NoRoute,
    /// A route failed and no usable replacement answered.
    #[error("the member is unreachable: its route failed and no newer one is published")]
    Unreachable,
    /// The call itself failed.
    #[error("the call failed: {0}")]
    Call(String),
}

#[derive(Clone, Copy)]
pub(super) enum FailedRoute<'a> {
    Version(&'a RouteVersion),
    /// Gossip selects a blob through the overlay without a DHT version.
    Blob(&'a [u8]),
}

fn control_type(payload: &[u8]) -> &'static str {
    if !tracing::enabled!(tracing::Level::DEBUG) {
        return "opaque";
    }
    match try_decode_community_envelope(payload)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::FrontierRequest { .. }) => "FrontierRequest",
        Some(ControlPayload::RequestMEK { .. }) => "RequestMEK",
        Some(ControlPayload::MekTransfer(_)) => "MekTransfer",
        _ => "opaque",
    }
}

impl HostGovernance {
    fn route_failed(&self, community: &str, record: &str, version: &RouteVersion) {
        if let Some(c) = self.ctx.communities.map().write().get_mut(community) {
            c.member_routes.set_usable(record, version, false);
        }
    }

    /// Call an effective device. Transport route failures try at most one
    /// forced read and one new blob per device record. An application
    /// answer ends the call, including a refusal.
    pub(crate) async fn call_member_route(
        &self,
        community: &str,
        member: &PseudonymKey,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        if !self.is_key_reader(community, member) {
            return Err(MemberCallError::NoRoute);
        }
        let control = control_type(&payload);
        let mut failure = MemberCallError::NoRoute;
        for binding in self.route_records(community, member) {
            match self
                .call_record(community, member, &binding, &payload, control)
                .await
            {
                Err(MemberCallError::NoRoute) => {}
                Err(MemberCallError::Unreachable) => failure = MemberCallError::Unreachable,
                answered => return answered,
            }
        }
        Err(failure)
    }

    async fn call_record(
        &self,
        community: &str,
        member: &PseudonymKey,
        binding: &RouteRecord,
        payload: &[u8],
        control: &str,
    ) -> Result<Vec<u8>, MemberCallError> {
        let record = &binding.record;
        // The typed call sees an unusable cached blob and gets its one
        // retry budget here, so a cold re-resolution cannot spend it twice.
        let initial = match self.cached_route(community, record) {
            Some(cached) => cached,
            None if !self.may_read_route(community, record, Instant::now()) => {
                trace_route(control, record, None, "capped");
                return Err(MemberCallError::Unreachable);
            }
            None => self
                .route_for_record(community, member, binding, control)
                .await
                .ok_or(MemberCallError::NoRoute)?,
        };
        if self
            .binding_of_record(community, record)
            .map(|(_, current)| current)
            .as_ref()
            != Some(binding)
        {
            return Err(MemberCallError::NoRoute);
        }
        trace_route(control, record, Some(&initial), "call");
        match self.call_route_typed(&initial.blob, payload.to_vec()).await {
            Ok(reply) => return Ok(reply),
            Err(error) if error.rereads_route() => {}
            Err(error) => return Err(MemberCallError::Call(error.to_string())),
        }
        let fresh = self
            .reread_route(
                community,
                member,
                binding,
                FailedRoute::Version(&initial),
                control,
            )
            .await
            .ok_or(MemberCallError::Unreachable)?;
        if self
            .binding_of_record(community, record)
            .map(|(_, current)| current)
            .as_ref()
            != Some(binding)
        {
            return Err(MemberCallError::NoRoute);
        }
        trace_route(control, record, Some(&fresh), "retry");
        match self.call_route_typed(&fresh.blob, payload.to_vec()).await {
            Ok(reply) => Ok(reply),
            Err(error) if error.rereads_route() => {
                self.route_failed(community, record, &fresh);
                if let Some(c) = self.ctx.communities.map().write().get_mut(community) {
                    if c.member_routes.is_current(record, &fresh) {
                        c.member_routes.finish_reread(record, false, Instant::now());
                    }
                }
                Err(MemberCallError::Unreachable)
            }
            Err(error) => Err(MemberCallError::Call(error.to_string())),
        }
    }

    pub(super) async fn reread_route(
        &self,
        community: &str,
        member: &PseudonymKey,
        binding: &RouteRecord,
        failed: FailedRoute<'_>,
        control: &str,
    ) -> Option<RouteVersion> {
        let record = &binding.record;
        let lock = self.route_read_lock(community, record)?;
        let _reading = lock.lock().await;
        let now = Instant::now();
        let (failed_blob, attempted) = match failed {
            FailedRoute::Version(version) => {
                self.route_failed(community, record, version);
                (version.blob.as_slice(), Some(version))
            }
            FailedRoute::Blob(blob) => (blob, None),
        };
        if let Some(fresh) = self.cached_route(community, record).filter(|version| {
            version.blob != failed_blob && !self.route_declared_dead(&version.blob)
        }) {
            trace_route(control, record, Some(&fresh), "cached_after_failure");
            return Some(fresh);
        }
        if !self.may_read_route(community, record, now) {
            trace_route(control, record, attempted, "capped");
            return None;
        }
        self.read_route_pinned(
            community,
            member,
            binding,
            Some(failed_blob),
            control,
            false,
        )
        .await;
        if !self.route_records(community, member).contains(binding) {
            return None;
        }
        let mut map = self.ctx.communities.map().write();
        let cache = &mut map.get_mut(community)?.member_routes;
        let fresh = cache
            .get(record)
            .filter(|current| {
                current.blob.as_slice() != failed_blob && !self.route_declared_dead(&current.blob)
            })
            .cloned();
        cache.finish_reread(record, fresh.is_some(), now);
        fresh
    }

    /// Re-resolve after a failed mesh send.
    pub async fn reread_member_route(
        &self,
        community: &str,
        member_hex: &str,
        failed_blob: &[u8],
    ) -> Option<Vec<u8>> {
        let member = super::moderation::pseudonym(member_hex)?;
        if !self.is_key_reader(community, &member) {
            return None;
        }
        let bindings = self.route_records(community, &member);
        for binding in bindings {
            if let Some(current) = self.cached_route(community, &binding.record) {
                if current.blob != failed_blob && !self.route_declared_dead(&current.blob) {
                    return Some(current.blob);
                }
            }
            if let Some(fresh) = self
                .reread_route(
                    community,
                    &member,
                    &binding,
                    FailedRoute::Blob(failed_blob),
                    "gossip",
                )
                .await
            {
                return Some(fresh.blob);
            }
        }
        None
    }

    pub(crate) async fn call_member_hex(
        &self,
        community: &str,
        member_hex: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        let member = super::moderation::pseudonym(member_hex).ok_or(MemberCallError::NoRoute)?;
        self.call_member_route(community, &member, payload).await
    }

    /// Call a member through the same route resolution as production.
    ///
    /// # Errors
    /// See [`MemberCallError`].
    #[cfg(feature = "netsim-entry")]
    pub async fn netsim_call_member(
        &self,
        community: &str,
        member_hex: &str,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, MemberCallError> {
        self.call_member_hex(community, member_hex, payload).await
    }

    /// Call a specific effective record in the simulator.
    ///
    /// # Errors
    /// See [`MemberCallError`].
    #[cfg(feature = "netsim-entry")]
    pub async fn netsim_call_member_record(
        &self,
        community: &str,
        record: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, MemberCallError> {
        let (author, binding) = self
            .binding_of_record(community, record)
            .ok_or(MemberCallError::NoRoute)?;
        self.call_record(community, &author, &binding, payload, control_type(payload))
            .await
    }
}
