//! Gossip, permissions, background tasks and hydration.
//!
//! The remainder of the trait surface — everything that is neither a
//! plain state access nor a direct DHT call.

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::CommunityDhtOpenSetup;
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_route::dead_routes::Observed;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

/// The shortest wait before a post-rebuild follow-up's turn.
const FOLLOW_UP_DELAY_MIN: std::time::Duration = std::time::Duration::from_secs(2);
/// The width, in milliseconds, of the jitter added to
/// [`FOLLOW_UP_DELAY_MIN`]: a turn waits 2 to 10 s.
const FOLLOW_UP_JITTER_MS: u64 = 8_000;

impl HostGovernance {
    // ---------- Gossip ----------

    /// Fan an envelope out over the community mesh, on both modes: the
    /// gossip worker signs it and sends it to [`Self::mesh_peers`] (plan
    /// E3.4 S5), every variant in the Cap'n Proto form desktop peers read
    /// (see `daemon::gossip`). Sent to the peers of the call: a caller that
    /// changes the roster next has already chosen whom it goes to.
    pub fn send_to_mesh_impl(&self, community_id: &str, envelope: &CommunityEnvelope) {
        crate::daemon::gossip::send(&self.ctx.gossip_tx, community_id, envelope);
    }

    /// Sign `envelope` and send it to the mesh now, awaiting every peer's
    /// send: the sends at a session's end (the quit's beat hint, a leave's
    /// notice), when the unlock scope a spawned fan-out runs in has closed
    /// (Linux W5b). Nothing is held without peers: no session follows to
    /// send it. The number of peers sent to.
    ///
    /// # Errors
    /// The envelope could not be signed or encoded.
    pub async fn send_to_mesh_awaited(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<usize, String> {
        let adapter = crate::daemon::gossip_adapter::DaemonGossipAdapter::new(
            std::sync::Arc::clone(&self.ctx),
        );
        rekindle_gossip::send_to_mesh_awaited(std::sync::Arc::new(adapter), community_id, envelope)
            .await
            .map_err(|e| e.to_string())
    }

    /// Open the community's gossip mesh as its membership is recorded, before
    /// the create or join publishes its first op (live A/B findings 7, 12):
    /// the mesh was registered only by `activate_community`, after the create
    /// or join returned, so every op the flow gossiped found no mesh. An open
    /// mesh with no online member yet takes the no-peers branch (PATH 2 has
    /// no durability; the op rides the governance-op stream to `M`). The
    /// shell opens its own services (`ShellHooks::community_records_ready`).
    ///
    /// The community's overlay is built here too, empty (its first presence
    /// poll fills it): a send before that poll is held for the drain
    /// (`enqueue_pending_mesh_impl`), never dropped for a missing overlay.
    /// Only a removed community's teardown leaves it without one.
    pub(crate) fn open_gossip_mesh(&self, community_id: &str) {
        if self.ctx.shell.is_some() {
            return;
        }
        if let Some(c) = self.ctx.communities.map().write().get_mut(community_id) {
            c.gossip
                .get_or_insert_with(crate::host::community::GossipOverlay::default);
        }
        if let Some(broadcast) = self.ctx.broadcast_mgr.read().as_ref() {
            broadcast.register_mesh(community_id);
        }
    }

    // ---------- Permissions ----------

    /// Reader-validates permission check against the merged CRDT state.
    ///
    /// No privileged shortcut for the daemon: it computes its own
    /// effective permissions exactly as any other peer would, from the
    /// governance state, and denies when the bits are absent. A daemon
    /// that trusted itself here would be a privileged node, which is the
    /// thing v2.0 removed.
    pub(super) fn require_permission_impl(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError> {
        let Some(state) = self.governance_state_impl(community_id) else {
            return Err(GovernanceRuntimeError::Adapter(
                "governance state not loaded for permission check".into(),
            ));
        };
        let Some(membership) = self.community_membership_impl(community_id) else {
            return Err(GovernanceRuntimeError::Adapter(
                "not a member of this community".into(),
            ));
        };
        let Some(pseudonym_hex) = membership.my_pseudonym_hex else {
            return Err(GovernanceRuntimeError::Adapter(
                "no pseudonym for this community".into(),
            ));
        };
        let pseudonym_bytes: [u8; 32] = hex::decode(&pseudonym_hex)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| GovernanceRuntimeError::Adapter("invalid pseudonym hex".into()))?;
        let pseudonym = PseudonymKey(pseudonym_bytes);

        let effective = rekindle_governance::permissions::compute_permissions(
            &pseudonym,
            None,
            &state,
            self.ctx.clock.now_secs(),
        );
        if rekindle_governance::permissions::has_capability(effective, perm_bits) {
            Ok(())
        } else {
            Err(GovernanceRuntimeError::PermissionDenied)
        }
    }

    // ---------- Join flow ----------

    /// `app_call` a peer through its advertised private route.
    ///
    /// **Unframed**, as the desktop sent it, a raw `app_call` (its
    /// governance adapter, deleted at E3.1 B2f). The daemon used to wrap the same
    /// bytes in a `TypeId::CommunityGovOp` frame and sign them as a
    /// `SignedPayload`, so the two shells could not have answered each
    /// other's peer calls — the frame is gone with the `GovernanceOp`
    /// mechanism it belonged to.
    ///
    /// The payload is the caller's business: this is the transport for
    /// the BootstrapBundle fetch (§14.4), which the architecture calls
    /// "convenience, not trust" — nothing here can be relied on for
    /// authority, and the bundle's contents are re-verified against the
    /// DHT regardless.
    pub(super) async fn app_call_peer_impl(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        self.call_route_typed(target_route_blob, payload)
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("app_call: {e}")))
    }

    /// [`Self::app_call_peer_impl`] keeping the transport's verdict on the
    /// route (plan E3.4 S4): a blob that does not import or a peer route the
    /// transport reports `RouteUnusable` (C9.1a; the transport has already
    /// invalidated its import, `broadcast/send.rs` `send_failure`) is
    /// [`RouteCallError::Unusable`], and a lost call is
    /// [`RouteCallError::TimedOut`]; after either a member call re-reads
    /// `M`/6 ([`RouteCallError::rereads_route`]). A blob Veilid declared dead
    /// (`RouteChange`, plan RV1g B1) is refused at import as `Unusable`, with
    /// no call. Timeouts declare nothing dead (B2 owed): an inviter that
    /// answers no call keeps its live route unrepublished (netsim
    /// `rejoin_after_kick`).
    pub(super) async fn call_route_typed(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, RouteCallError> {
        let node = self
            .transport()
            .map_err(|e| RouteCallError::Failed(e.to_string()))?;
        let target = node
            .import_route(target_route_blob)
            .map_err(|_| RouteCallError::Unusable)?;
        let result = node
            .caller()
            .call_community_envelope(&target, target_route_blob, payload)
            .await;
        result.map_err(|e| match e {
            rekindle_transport::TransportError::RouteUnusable { .. } => RouteCallError::Unusable,
            rekindle_transport::TransportError::Timeout { reason, .. } => {
                RouteCallError::TimedOut(reason)
            }
            other => RouteCallError::Failed(other.to_string()),
        })
    }

    /// Whether `blob` is a peer route the transport knows dead (plan RV1g
    /// B1): no call is made into it while it is.
    pub(crate) fn route_declared_dead(&self, blob: &[u8]) -> bool {
        self.transport()
            .is_ok_and(|node| node.liveness().route_declared_dead(blob))
    }

    /// A read of a peer's record returned route `blob` at DHT `seq`: a
    /// strictly newer read of a dead blob makes it usable again (plan RV1g
    /// B3), and the same seq one re-fetch gap after its death asks for a
    /// probe (plan RH1r-c S2/S3, `route_probe`).
    pub(crate) fn route_read_at(&self, blob: &[u8], seq: u64) -> Observed {
        self.transport().map_or(Observed::NotDead, |node| {
            node.liveness().route_read_at(blob, seq)
        })
    }

    // ---------- Hydration ----------

    pub(super) fn list_communities_for_dht_open_impl(&self) -> Vec<CommunityDhtOpenSetup> {
        self.store_memberships()
            .iter()
            .map(|m| {
                // Under `o_cnt: 0` there is no registry owner; a member
                // writes its registry slot with its derived slot keypair.
                let slot_writer = m.slot_seed.as_ref().and_then(|seed| {
                    rekindle_crypto::keypair_text::slot_keypair_text(seed, m.slot_index).ok()
                });
                CommunityDhtOpenSetup {
                    id: m.anchor.clone(),
                    anchor: m.anchor.clone(),
                    registry_key: (!m.registry_key.is_empty()).then(|| m.registry_key.clone()),
                    registry_writer: slot_writer,
                }
            })
            .collect()
    }

    /// Invite records this identity published and must keep warm: the
    /// `secrets_record_key` of every unexpired invite we created, from the
    /// merged governance state. Only invites we authored are in our local
    /// record store, so re-opening them is a local hit that rehydrates them.
    pub(super) fn list_my_active_invite_secret_keys_impl(&self) -> Vec<String> {
        let now = self.ctx.clock.now_secs();
        let governed: Vec<(String, GovernanceState)> = self
            .ctx
            .communities
            .map()
            .read()
            .iter()
            .filter_map(|(id, c)| Some((id.clone(), c.governance_state.clone()?)))
            .collect();
        let mut keys = Vec::new();
        for (community_id, gov) in governed {
            let Some(mine) = self
                .community_membership_impl(&community_id)
                .and_then(|m| m.my_pseudonym_hex)
            else {
                continue;
            };
            for invite in gov.invites.values() {
                if hex::encode(invite.creator_pseudonym.0) != mine
                    || invite.expires_at.is_some_and(|exp| exp <= now)
                    || invite.secrets_record_key.is_empty()
                {
                    continue;
                }
                keys.push(invite.secrets_record_key.clone());
            }
        }
        keys
    }

    /// Project the merged state, save its snapshot for the next login's
    /// restore, then schedule the follow-up that republishes our frontier
    /// and moves the watch ring.
    ///
    /// The frontier write and the ring move are DHT round trips, so they
    /// run outside the rebuild: a rebuild a `BootstrapRequest` triggers
    /// (`op_log::receive_op`) is answered within the app_call deadline.
    pub(super) async fn apply_governance_rebuild_result_impl(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
    ) {
        self.set_governance_state_impl(community_id, gov_state);
        self.persist_governance_snapshot(community_id).await;
        // Moderator deletes are applied from every merge (plan E3.3 M4).
        self.apply_admin_deletes(community_id).await;
        self.schedule_follow_up(community_id);
    }

    /// The post-rebuild follow-up (S7l): one task per community, on the
    /// community's scope so leave stops it, waiting a jittered
    /// [`FOLLOW_UP_DELAY_MIN`] plus up to [`FOLLOW_UP_JITTER_MS`] before each turn
    /// (the threat model refuses deterministic timing: a write right after
    /// each merge tells the DHT when this member merged), so the rebuilds
    /// that finish meanwhile cost one more turn, not one write each.
    pub(super) fn schedule_follow_up(&self, community_id: &str) {
        let communities = &self.ctx.communities;
        if !communities.map().read().contains_key(community_id)
            || !communities.mark_follow_up(community_id)
        {
            return;
        }
        let host = self.clone();
        let community = community_id.to_owned();
        let span = tracing::Span::current();
        let started = self.ctx.community_scope(community_id).spawn_with_token(
            "frontier and ring after a rebuild",
            move |stop| {
                tracing::Instrument::instrument(
                    async move {
                        loop {
                            let delay = host.follow_up_delay();
                            if stop
                                .run_until_cancelled(tokio::time::sleep(delay))
                                .await
                                .is_none()
                                || !host.ctx.communities.take_follow_up(&community)
                            {
                                return;
                            }
                            host.republish_own_frontier(&community).await;
                            host.rearm_watch_ring(&community).await;
                        }
                    },
                    span,
                )
            },
        );
        if let Err(closed) = started {
            tracing::debug!(community_id, error = %closed, "follow-up dropped");
            communities.stop_follow_up(community_id);
        }
    }

    /// The member card refresh after a rebuild (plan JP1): one task per
    /// community on the community's scope (leave or lock stops it at its
    /// next read), so a rebuild never waits on the card reads; the
    /// rebuilds that finish while it runs cost it one more turn, which
    /// reads only the cards still unnamed.
    pub(super) fn schedule_card_refresh(&self, community_id: &str) {
        let communities = &self.ctx.communities;
        if !communities.map().read().contains_key(community_id)
            || !communities.mark_card_refresh(community_id)
        {
            return;
        }
        let host = self.clone();
        let community = community_id.to_owned();
        let span = tracing::Span::current();
        let started = self.ctx.community_scope(community_id).spawn_with_token(
            "member cards after a rebuild",
            move |stop| {
                tracing::Instrument::instrument(
                    async move {
                        while host.ctx.communities.take_card_refresh(&community) {
                            let refresh =
                                rekindle_governance_runtime::member_card::refresh_member_cards(
                                    &host, &community,
                                );
                            if stop.run_until_cancelled(refresh).await.is_none() {
                                host.ctx.communities.stop_card_refresh(&community);
                                return;
                            }
                        }
                    },
                    span,
                )
            },
        );
        if let Err(closed) = started {
            tracing::debug!(community_id, error = %closed, "card refresh dropped");
            communities.stop_card_refresh(community_id);
        }
    }

    /// [`FOLLOW_UP_DELAY_MIN`] plus a jitter below [`FOLLOW_UP_JITTER_MS`]
    /// from the RNG port.
    fn follow_up_delay(&self) -> std::time::Duration {
        let mut bytes = [0u8; 8];
        self.ctx.rng.fill(&mut bytes);
        FOLLOW_UP_DELAY_MIN
            + std::time::Duration::from_millis(u64::from_le_bytes(bytes) % FOLLOW_UP_JITTER_MS)
    }

    /// The community's projected governance into SQLite
    /// (`repo::communities::snapshot`), awaited so a restart restores it.
    pub(crate) async fn persist_governance_snapshot(&self, community_id: &str) {
        let Some(owner) = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())
        else {
            return;
        };
        let Ok(db) = self.ctx.db.current() else {
            return;
        };
        let Some(snapshot) = self.ctx.communities.governance_snapshot(community_id) else {
            tracing::warn!(community_id, "no governance snapshot to persist");
            return;
        };
        let community = community_id.to_string();
        let saved = db
            .call(move |conn| {
                rekindle_db::repo::communities::snapshot::replace(
                    conn, &owner, &community, &snapshot,
                )
            })
            .await;
        if let Err(error) = saved {
            tracing::warn!(community_id, %error, "rebuilt governance snapshot not persisted");
        }
    }

    // ---------- Background tasks ----------

    /// Queue the forward-secrecy rotation that a ban requires.
    ///
    /// A banned member still holds the current MEK, so the ban is only
    /// half of the removal until the key changes. This used to trace a
    /// line and return, which meant the daemon track never completed
    /// that half.
    ///
    /// Fire-and-forget by contract: the ban entry has already been
    /// written and must not be undone by a rotation failure, so a full
    /// queue or a stopped worker is logged rather than propagated.
    pub(super) fn spawn_text_mek_rotation_for_ban_impl(
        &self,
        community_id: &str,
        banned_pseudonym_hex: &str,
    ) {
        let request = crate::daemon::mek_rotation::MekRotationRequest::departure(
            community_id,
            banned_pseudonym_hex,
        );
        if self.ctx.mek_rotation_tx.send(request).is_err() {
            tracing::warn!(
                community_id,
                "MEK rotation worker is gone — ban did not rotate the key"
            );
        }
    }
}

/// How a call to a peer route failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RouteCallError {
    /// The blob did not import or the transport reported `RouteUnusable`.
    #[error("the route is unusable")]
    Unusable,
    /// The call's answer did not come back (Veilid's `Timeout` or the
    /// transport's bound): in Veilid 0.5.8 a call into a route its owner
    /// released is lost this way, never answered `RouteUnusable`, since
    /// an importer is never told of the release
    /// (`route_spec_store/mod.rs:235-251`).
    #[error("the call timed out: {0}")]
    TimedOut(String),
    /// Any other failure.
    #[error("{0}")]
    Failed(String),
}

impl RouteCallError {
    /// Whether the failure may mean the member's route is gone, so its
    /// `M`/6 is read once more (plan E3.4 S4, lead decision): an unusable
    /// route or a lost call. Any other failure keeps the route.
    #[must_use]
    pub(crate) fn rereads_route(&self) -> bool {
        matches!(self, Self::Unusable | Self::TimedOut(_))
    }
}
