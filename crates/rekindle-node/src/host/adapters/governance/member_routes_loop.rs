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

use rekindle_codec::member_record::{BeatState, RecordScope};
use rekindle_governance_runtime::member_card::{write_own_beat, OwnBeat, OwnRecord};
use rekindle_governance_runtime::member_routes::write_routes;
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_presence::community::seal_extras;
use rekindle_records::port::{RouteClass, RouteState};
use tokio_util::sync::CancellationToken;

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
                    Ok(()) => host.note_published_route(&community, &keys.record, &blob),
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
    ) -> Result<(), String> {
        let port = self.record_port().map_err(|e| e.to_string())?;
        write_routes(
            &*port,
            &keys.own(community_id),
            blob,
            self.ctx.clock.now_ms(),
        )
        .await
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
        let port = self.record_port().map_err(|e| e.to_string())?;
        let seq = self
            .next_beat_seq(community_id)
            .await
            .map_err(|e| format!("beat seq: {e}"))?;
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
            let session = presence::own_session(&self.ctx, community_id);
            let policy = presence::presence_policy(&self.ctx, community_id);
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
        let seq = self
            .write_own_beat(community_id, BeatState::Offline)
            .await?;
        tracing::debug!(seq, "quit beat written; gossiping its hint");
        let keys = self.own_record_keys(community_id)?;
        let hint = rekindle_codec::member_record::head_advanced::HeadAdvanced {
            author: keys.pseudonym.verifying_key().to_bytes(),
            member_record: keys.record.clone(),
            subkey: rekindle_types::dht_layout::member_record::BEAT,
            seq,
        };
        let hint = hint
            .encode(community_id, &keys.pseudonym)
            .map_err(|e| format!("quit hint: {e}"))?;
        let envelope = rekindle_codec::community::envelope::CommunityEnvelope::control(
            rekindle_codec::community::envelope::ControlPayload::HeadAdvanced { hint },
        );
        self.send_before_teardown(community_id, &envelope).await;
        Ok(())
    }

    /// Our member record in `community_id` and the keys that write and sign
    /// it.
    pub(super) fn own_record_keys(&self, community_id: &str) -> Result<OwnRecordKeys, String> {
        let record = self
            .own_member_record(community_id)
            .ok_or("no member record of ours")?;
        let writer = self.member_record_writer(community_id)?;
        let secret = self
            .identity_secret_impl()
            .ok_or("no identity is unlocked")?;
        let pseudonym =
            rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);
        Ok(OwnRecordKeys {
            record,
            writer,
            pseudonym,
        })
    }
}

/// What [`OwnRecord`] borrows.
pub(super) struct OwnRecordKeys {
    record: String,
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
