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

use std::time::Duration;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::member_record::head_advanced::HeadAdvanced;
use rekindle_codec::member_record::{routes::decode_member_routes, BeatState, RecordScope};
use rekindle_crypto::keypair_text::{parse_keypair_text, public_key_text};
use rekindle_governance::device_judge::{judge_key, KeyJudgement};
use rekindle_governance_runtime::member_card::SubkeyWriteError;
use rekindle_governance_runtime::member_card::{write_own_beat, OwnBeat, OwnRecord};
use rekindle_governance_runtime::member_routes::write_routes;
use rekindle_governance_runtime::member_routes::{read_member_route_at, RoutePin};
use rekindle_governance_runtime::unlock_barrier::BarrierPhase;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_presence::community::seal_extras;
use rekindle_records::port::{RouteClass, RouteState};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_transport::route_trace::blob_tag;
use rekindle_types::dht_layout::member_record::{BEAT, CARD, ROUTES};
use rekindle_types::id::PseudonymKey;
use rekindle_utils::log_scrub::tag;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use super::HostGovernance;
use crate::host::adapters::presence;

const ROUTE_RETRY: Duration = Duration::from_secs(10);

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let routes = match host.transport() {
        Ok(transport) => transport.routes(),
        Err(error) => {
            tracing::warn!(%error, "member routes: no transport, routes not published");
            return;
        }
    };
    let mut general = routes.own_state(RouteClass::General);
    let changed = &host.ctx.communities.member_routes_changed;
    loop {
        let state = general.borrow_and_update().clone();
        let blob = match state {
            RouteState::Available { blob } => Some(blob),
            _ => None,
        };
        let mut retry = false;
        if let Some(blob) = blob {
            for community in host.sync_targets() {
                if stop.is_cancelled() {
                    return;
                }
                let Ok(keys) = host.own_record_keys(&community) else {
                    continue;
                };
                if !host.route_needs_write(&community, &keys.record, &blob) {
                    continue;
                }
                match host.publish_own_routes(&community, &keys, &blob).await {
                    Ok(true) => host.note_published_route(&community, &keys.record, &blob),
                    Ok(false) => {}
                    Err(error) => {
                        retry = true;
                        tracing::warn!(%community, %error, "member routes not written");
                    }
                }
            }
        }
        tokio::select! {
            () = stop.cancelled() => return,
            result = general.changed() => if result.is_err() { return },
            () = changed.notified() => {},
            () = tokio::time::sleep(ROUTE_RETRY), if retry => {},
        }
    }
}

impl HostGovernance {
    pub(super) fn begin_member_record(&self, community: &str) {
        if let Some(state) = self.ctx.communities.map().write().get_mut(community) {
            state.initializing_member_routes = true;
            state.written_member_route = None;
        }
    }

    pub(super) fn finish_member_record(&self, community: &str, record: &str, blob: &[u8]) {
        if let Some(state) = self.ctx.communities.map().write().get_mut(community) {
            state.initializing_member_routes = false;
            state.written_member_route = Some((record.to_owned(), blob.to_vec()));
        }
        self.ctx.communities.member_routes_changed.notify_one();
    }

    fn route_needs_write(&self, community: &str, record: &str, blob: &[u8]) -> bool {
        self.ctx
            .communities
            .map()
            .read()
            .get(community)
            .is_some_and(|state| {
                !state.initializing_member_routes
                    && state.conflicted_member_route.as_deref() != Some(record)
                    && !state.written_member_route.as_ref().is_some_and(
                        |(written_record, written_blob)| {
                            written_record == record && written_blob == blob
                        },
                    )
            })
    }

    fn note_published_route(&self, community: &str, record: &str, blob: &[u8]) {
        if let Some(state) = self.ctx.communities.map().write().get_mut(community) {
            if !state.initializing_member_routes {
                state.written_member_route = Some((record.to_owned(), blob.to_vec()));
            }
        }
    }

    /// Our member record in `community_id`, as the join or create recorded
    /// it (`communities.my_member_record`, ADR 0011): never our merged
    /// roster's entry, which our own `MemberLeft` removes while the leave's
    /// frontier write and our outbox's head are still ours to land.
    pub(crate) fn own_member_record(&self, community_id: &str) -> Option<String> {
        self.ctx
            .communities
            .map()
            .read()
            .get(community_id)?
            .my_member_record
            .clone()
    }

    /// Whether our own pseudonym departed `community_id` in our merged
    /// state (our `MemberLeft`, a kick or a ban): no beat is ours to write.
    pub(crate) fn own_departed(&self, community_id: &str) -> bool {
        let map = self.ctx.communities.map().read();
        map.get(community_id).is_some_and(|c| {
            let me = c
                .my_pseudonym_key
                .as_deref()
                .and_then(super::ops::pseudonym_of);
            match (me, c.governance_state.as_ref()) {
                (Some(me), Some(state)) => state.is_departed(&me),
                _ => false,
            }
        })
    }

    /// Write `blob` to our `M`/6 in `community_id`.
    ///
    /// # Errors
    /// We hold no member record or keys there, or the write failed.
    async fn publish_own_routes(
        &self,
        community_id: &str,
        keys: &OwnRecordKeys,
        blob: &[u8],
    ) -> Result<bool, String> {
        let port = self.record_port().map_err(|e| e.to_string())?;
        let result = write_routes(
            &*port,
            &keys.own(community_id),
            blob,
            self.ctx.clock.now_ms(),
        )
        .await;
        if !self.is_publication_record(community_id, &keys.record) {
            self.stop_member_record_republishes(&keys.record);
            return Ok(false);
        }
        let rival = match result {
            Ok(()) => return Ok(true),
            Err(SubkeyWriteError::Failed(error)) => return Err(error),
            Err(SubkeyWriteError::Superseded(rival)) => rival,
        };
        let record = keys.record.as_str();
        let at = RecordScope {
            community_id,
            member_record: record,
        };
        let member = PseudonymKey(keys.pseudonym.verifying_key().to_bytes());
        let verified_rival = (rival.writer == keys.owner)
            .then(|| decode_member_routes(&rival.data, at, &member.0).ok())
            .flatten();
        if verified_rival
            .as_ref()
            .is_some_and(|routes| routes.general == blob)
        {
            return Ok(true);
        }

        port.cancel_republish(record, &[ROUTES]);
        let refreshed = read_member_route_at(
            &*port,
            community_id,
            record,
            &member,
            RoutePin::Joined(&keys.owner),
        )
        .await;
        let still_eligible = self.is_publication_record(community_id, record);
        if still_eligible
            && refreshed.as_ref().is_ok_and(|read| {
                read.as_ref()
                    .is_some_and(|(seq, current)| *seq > u64::from(rival.seq) && current == blob)
            })
        {
            return Ok(true);
        }
        if let Some(state) = self.ctx.communities.map().write().get_mut(community_id) {
            state.conflicted_member_route = Some(record.to_owned());
        }
        tracing::warn!(
            community = community_id,
            control_type = "publish",
            record_tag = %tag(record),
            route_tag = %blob_tag(blob),
            rival_route_tag = verified_rival.map(|routes| blob_tag(&routes.general)),
            seq = rival.seq,
            still_eligible,
            refreshed_seq = refreshed.ok().flatten().map(|(seq, _)| seq),
            read_disposition = "superseded",
            "member route publication conflict"
        );
        Ok(false)
    }

    pub(super) fn stop_member_record_republishes(&self, record: &str) {
        if let Ok(port) = self.record_port() {
            port.cancel_republish(record, &[CARD, ROUTES]);
        }
    }

    pub(super) fn update_publication_binding(&self, community: &str, previous: Option<&str>) {
        let current = self.own_record_keys(community).ok().map(|keys| keys.record);
        if current.as_deref() == previous {
            return;
        }
        if let Some(record) = previous {
            self.stop_member_record_republishes(record);
        }
        if let Some(record) = current {
            self.wake_device_publication(community);
            // Rule 7 readmission can make a device effective after its
            // re-enrolment command has returned.
            if self
                .own_member_record(community)
                .is_some_and(|base| base != record)
            {
                self.refresh_bound_device_card(community, record);
            }
        }
    }

    pub(super) fn wake_device_publication(&self, community: &str) {
        self.ctx.communities.member_routes_changed.notify_one();
        self.ctx.communities.member_history_changed.notify_one();
        let Some(engine) = self.ctx.own_presence.beats.read().clone() else {
            return;
        };
        let community = community.to_owned();
        self.ctx
            .community_scope(&community)
            .spawn_or_drop("device publication beat", async move {
                if let Err(error) = engine.refresh_community(&community).await {
                    tracing::debug!(%community, %error, "device publication beat skipped");
                }
            });
    }

    /// Write our beat saying `state` to our `M`/1 in `community_id`, under
    /// the next `communities.beat_seq` (plan E3.4 S2, the beat engine's
    /// sink, `host::adapters::presence`); the subkey's DHT seq it left.
    ///
    /// # Errors
    /// We hold no member record or keys there, the seq was not persisted,
    /// or the write failed.
    pub(crate) async fn write_own_beat(
        &self,
        community_id: &str,
        state: BeatState,
    ) -> Result<u64, String> {
        let keys = self.own_record_keys(community_id)?;
        self.write_beat_for_record(community_id, &keys, state).await
    }

    async fn write_beat_for_record(
        &self,
        community_id: &str,
        keys: &OwnRecordKeys,
        state: BeatState,
    ) -> Result<u64, String> {
        let port = self.record_port().map_err(|e| e.to_string())?;
        let seq = self
            .next_beat_seq(community_id)
            .await
            .map_err(|e| format!("beat seq: {e}"))?;
        let (session, policy) = presence::own_session_with_policy(&self.ctx, community_id);
        let state = if state.is_present() && !policy.share_online {
            BeatState::Offline
        } else {
            state
        };
        // A present beat says our voice channel (the shell's until E4.1, the
        // Host's own on `rekindled`)
        // and carries our extras sealed from our own session and consent
        // (the Host's, plan E3.4 S8-pre); an offline or departed one says
        // nothing more than its state.
        let (voice_channel, extras) = if state.is_present() {
            let key = self.community_key(community_id);
            let at = RecordScope {
                community_id,
                member_record: &keys.record,
            };
            let extras = seal_extras(session, &policy, key.as_ref(), at, &*self.ctx.rng);
            let voice_channel =
                presence::session_presence(&self.ctx).active_voice_channel(community_id);
            (voice_channel, extras)
        } else {
            (None, None)
        };
        let beat = OwnBeat {
            seq,
            state,
            now_ms: self.ctx.clock.now_ms(),
            voice_channel,
            extras,
        };
        if !self.is_publication_record(community_id, &keys.record) {
            return Err("device publication binding changed".into());
        }
        write_own_beat(&*port, &keys.own(community_id), &beat).await
    }

    /// Our quit (plan E3.4 "Constant rate": an explicit `offline` beat
    /// plus a `HeadAdvanced` hint): write our `Offline` beat in
    /// `community_id`, then gossip the hint for it ourselves, so every
    /// member reads it within one gossip round instead of after a ring
    /// watcher's watch notification and relay. Awaited until the fan-out
    /// holds its peers, since the teardown removes the overlay next.
    ///
    /// # Errors
    /// The beat was not written (no hint is sent).
    pub(crate) async fn write_quit_beat(&self, community_id: &str) -> Result<(), String> {
        if let Some(hint) = self
            .write_beat_hint(community_id, BeatState::Offline)
            .await?
        {
            self.send_before_teardown(community_id, &hint).await;
        }
        Ok(())
    }

    /// Write a refreshed or quit beat and sign a hint for the exact write.
    pub(crate) async fn write_beat_hint(
        &self,
        community_id: &str,
        state: BeatState,
    ) -> Result<Option<CommunityEnvelope>, String> {
        let keys = self.own_record_keys(community_id)?;
        let seq = self
            .write_beat_for_record(community_id, &keys, state)
            .await?;
        if !self.is_publication_record(community_id, &keys.record) {
            return Ok(None);
        }
        let hint = HeadAdvanced {
            author: keys.pseudonym.verifying_key().to_bytes(),
            member_record: keys.record,
            subkey: BEAT,
            seq,
        }
        .encode(community_id, &keys.pseudonym)
        .map_err(|e| format!("beat hint: {e}"))?;
        Ok(Some(CommunityEnvelope::control(
            ControlPayload::HeadAdvanced { hint },
        )))
    }

    pub(super) fn is_publication_record(&self, community: &str, record: &str) -> bool {
        self.own_record_keys(community)
            .is_ok_and(|keys| keys.record == record)
    }

    /// The one gate for ordinary member-record publication. Fresh record
    /// initialization and the retirement pointer have their own writers.
    pub(super) fn own_record_keys(&self, community: &str) -> Result<OwnRecordKeys, String> {
        if self.ctx.communities.unlock_barriers().of(community).phase() != BarrierPhase::Passed {
            return Err("device publication is waiting for catch-up".into());
        }
        if self
            .ctx
            .communities
            .map()
            .read()
            .get(community)
            .is_some_and(|state| state.initializing_member_routes)
        {
            return Err("the member record is being initialized".into());
        }
        let state = self
            .governance_state_impl(community)
            .ok_or("no governance state")?;
        let secret = self
            .identity_secret_impl()
            .ok_or("no identity is unlocked")?;
        let pseudonym =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community);
        let author = PseudonymKey(pseudonym.verifying_key().to_bytes());
        let key = self
            .device_op_key(community)
            .map_err(|error| error.to_string())?
            .ok_or("no device key is held")?;
        let device = SigningKey::from_bytes(&key).verifying_key().to_bytes();
        let (record, writer) = self.own_op_record_writer(community)?;
        let membership = rekindle_governance_runtime::op_log::own_member(self, community, &state)
            .map_err(|error| error.to_string())?
            .record
            .ok_or("no current membership record")?;
        if state.is_departed(&author) {
            return Err("our membership ended".into());
        }
        let devices = state
            .devices
            .member(&author)
            .ok_or("no device endorsements")?;
        let endorsed = devices
            .endorsement_for(&author, community, &membership, &device)
            .ok_or("our device has no current endorsement")?;
        let (public, mut owner_secret) =
            parse_keypair_text(&writer).map_err(|error| error.to_string())?;
        owner_secret.zeroize();
        let owner = public_key_text(&public);
        if endorsed.device_record != record
            || endorsed.device_record_owner != owner
            || judge_key(devices, &author, community, &membership, &device, u64::MAX)
                != KeyJudgement::Effective
        {
            return Err("our device does not own an effective publication binding".into());
        }
        Ok(OwnRecordKeys {
            record,
            owner,
            writer,
            pseudonym,
        })
    }
}

/// What [`OwnRecord`] borrows.
pub(super) struct OwnRecordKeys {
    pub(super) record: String,
    pub(super) owner: String,
    writer: zeroize::Zeroizing<String>,
    pseudonym: rekindle_secrets::ed25519_dalek::SigningKey,
}

impl OwnRecordKeys {
    pub(super) fn own<'a>(&'a self, community_id: &'a str) -> OwnRecord<'a> {
        OwnRecord {
            community_id,
            record: &self.record,
            writer: self.writer.as_str(),
            pseudonym: &self.pseudonym,
        }
    }
}
