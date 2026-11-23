//! The `mek_transfer` stream (plan E3.2 S7l, D16, D22): a community key
//! sealed to one member is a row of the stream of (community, member),
//! its payload the route the member gave and the `MekTransfer` envelope
//! already sealed to its pseudonym (never the key itself). Each attempt
//! `app_call`s the member's current route (the one its presence carries,
//! else the one it gave), and only its `MekTransferAck` whose
//! key-confirmation tag verifies under that key (review S7m part 1)
//! settles the row `delivered`; anything else is tried again until the
//! deadline. A member who is no longer a key reader settles its row
//! unsent, and the member's departure, or ours, discards the stream.
//! A row whose key a newer held generation replaced settles unsent too
//! (D22 per-kind supersession, as `media_key`): the confirmed rotation
//! that raised the generation (ADR 0018) queued one row at the new one
//! for every member still owed a key (`HostGovernance::
//! requeue_key_transfers`, review S7m part 1 F2).

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxDeps as _, OutboxError, OutboxRow, StreamKind,
};
use rekindle_types::channel_keys::KeyScope;

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::adapters::governance::member_route_cache::MemberCallError;
use crate::host::adapters::governance::HostGovernance;

/// The reason a row whose member departed settles with.
pub const DEPARTED: &str = "departed";

/// The reason a row whose key a newer held generation replaced settles with.
pub const SUPERSEDED: &str = "a newer key";

fn scope(community: &str, member: &str) -> String {
    format!("{community}:{member}")
}

/// The payload: `route length (u32 BE) ‖ route ‖ sealed envelope`.
fn encode(route: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(4 + route.len() + sealed.len());
    out.extend_from_slice(&u32::try_from(route.len()).ok()?.to_be_bytes());
    out.extend_from_slice(route);
    out.extend_from_slice(sealed);
    Some(out)
}

fn decode(payload: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = usize::try_from(u32::from_be_bytes(payload.get(..4)?.try_into().ok()?)).ok()?;
    Some((payload.get(4..4 + len)?, payload.get(4 + len..)?))
}

/// What the sealed envelope names: its key's scope and generation and
/// its sender (our pseudonym, hex).
struct SealedKey {
    scope: KeyScope,
    generation: u64,
    sender_hex: String,
}

fn sealed_key(sealed: &[u8]) -> Option<SealedKey> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(sealed)
        .ok()
        .flatten()
        .and_then(CommunityEnvelope::into_control)
    {
        Some(ControlPayload::MekTransfer(p)) => Some(SealedKey {
            scope: KeyScope::from_wire(p.channel_id.as_deref())?,
            generation: p.generation,
            sender_hex: p.sender_pseudonym,
        }),
        _ => None,
    }
}

/// Sends one `mek_transfer` row (the stream's [`StreamSender`]).
pub(crate) struct MekTransferSender(pub(crate) HostGovernance);

#[async_trait]
impl StreamSender for MekTransferSender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        let Some((community, member)) = row
            .stream
            .strip_prefix("mek_transfer:")
            .and_then(|s| s.rsplit_once(':'))
        else {
            return AttemptOutcome::Wedged(format!("not a key transfer stream: {}", row.stream));
        };
        let Some((route, sealed)) = decode(&row.payload) else {
            return AttemptOutcome::Wedged("malformed key transfer row".into());
        };
        let Some(sealed_key) = sealed_key(sealed) else {
            return AttemptOutcome::Wedged("the row holds no MekTransfer".into());
        };
        let generation = sealed_key.generation;
        if !self.0.member_reads_keys(community, member) {
            return AttemptOutcome::Superseded(DEPARTED.to_owned());
        }
        // A member whose role reads history is owed every generation (D16,
        // `key_history.rs`); only one without it is owed just the newest.
        if (self.0.current_key_generation(community, sealed_key.scope))
            .is_some_and(|current| current > generation)
            && !self.0.member_reads_history(community, member)
        {
            return AttemptOutcome::Superseded(SUPERSEDED.to_owned());
        }
        // The ack's tag is checked under our own copy of the sealed key.
        let Some(key) = self.0.held_key(community, sealed_key.scope, generation) else {
            return AttemptOutcome::Wedged("the sealed key is no longer held".into());
        };
        let expected = rekindle_mek_rotation::ExpectedAck {
            community_id: community,
            inviter_hex: &sealed_key.sender_hex,
            joiner_hex: member,
            scope: sealed_key.scope,
            mek: &key,
        };
        // The joiner's current `M`/6 route (plan E3.4 S4: one re-read
        // after `RouteUnusable`); the route its `BootstrapRequest` sent
        // while its record holds none yet.
        let call = match self
            .0
            .call_member_hex(community, member, sealed.to_vec())
            .await
        {
            Err(MemberCallError::NoRoute) => self
                .0
                .call_member(route, sealed.to_vec())
                .await
                .map_err(MemberCallError::Call),
            other => other,
        };
        match call {
            Ok(reply) if rekindle_mek_rotation::is_mek_transfer_ack(&expected, &reply) => {
                // A removal that merged while the call was out cannot
                // recall the bytes (review S7m part 1 F3): its generation
                // is the one the departure rotation retires (ADR 0018).
                if !self.0.member_reads_keys(community, member) {
                    tracing::warn!(
                        community,
                        member,
                        generation,
                        "a key delivery crossed the member's removal"
                    );
                }
                tracing::info!(
                    community,
                    member,
                    generation,
                    "community key delivered (D16)"
                );
                // Delivered: its copy leaves our key drop (plan RV2d), so
                // no wrapped key of a settled row stays at rest.
                self.0.spawn_key_drop_refresh(community, member);
                AttemptOutcome::Receipted
            }
            Ok(_) => AttemptOutcome::Deferred,
            Err(error) => {
                tracing::debug!(community, member, %error, "key transfer not answered");
                AttemptOutcome::Deferred
            }
        }
    }
}

/// The unlocked identity's `mek_transfer` streams.
pub struct MekTransfers(Arc<Outbox<HostOutbox>>);

impl MekTransfers {
    /// Commit `sealed`, a `MekTransfer` envelope sealed to `member`, on
    /// the stream of (`community`, `member`) as `client_id`, to be called
    /// to `route` (or the member's newer one). A `client_id` whose row is
    /// still kept, in whatever state, is left as it is: a joiner's repeated
    /// bootstrap asks for the same key again (one row per member and
    /// generation, D16); `false` then.
    ///
    /// # Errors
    /// The storage failed, or the route is too long to frame.
    pub async fn queue(
        &self,
        community: &str,
        member: &str,
        client_id: &str,
        route: &[u8],
        sealed: &[u8],
    ) -> Result<bool, OutboxError> {
        let payload = encode(route, sealed)
            .ok_or_else(|| OutboxError::Storage("key transfer route too long".into()))?;
        let entry = Entry {
            kind: StreamKind::MekTransfer,
            scope: scope(community, member),
            client_id: client_id.to_owned(),
            payload,
            deadline: Deadline::Horizon,
        };
        let queued = self.0.deps().insert_once(self.0.new_row(entry)).await?;
        if queued {
            self.0.wake();
        }
        Ok(queued)
    }

    /// The unsettled rows of `community`'s streams, every member's.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn rows(&self, community: &str) -> Result<Vec<OutboxRow>, OutboxError> {
        (self.0.deps())
            .prefix_rows(&StreamKind::MekTransfer.stream(&scope(community, "")))
            .await
    }

    /// The (member, sealed envelope) of every unsettled row of
    /// `community`: what our key drop holds (plan RV2d).
    ///
    /// # Errors
    /// The storage failed.
    pub async fn sealed(&self, community: &str) -> Result<Vec<(String, Vec<u8>)>, OutboxError> {
        let prefix = format!("mek_transfer:{community}:");
        Ok(self
            .rows(community)
            .await?
            .iter()
            .filter_map(|row| {
                let member = row.stream.strip_prefix(&prefix)?;
                let (_, sealed) = decode(&row.payload)?;
                Some((member.to_owned(), sealed.to_vec()))
            })
            .collect())
    }

    /// The (member, route) of every row of `community` whose sealed key
    /// is of `key_scope` and older than `generation`: the members a
    /// rotation to `generation` still owes a key.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn behind(
        &self,
        community: &str,
        key_scope: KeyScope,
        generation: u64,
    ) -> Result<Vec<(String, Vec<u8>)>, OutboxError> {
        let prefix = format!("mek_transfer:{community}:");
        Ok(self
            .rows(community)
            .await?
            .iter()
            .filter_map(|row| {
                let member = row.stream.strip_prefix(&prefix)?;
                let (route, sealed) = decode(&row.payload)?;
                let key = sealed_key(sealed)?;
                (key.scope == key_scope && key.generation < generation)
                    .then(|| (member.to_owned(), route.to_vec()))
            })
            .collect())
    }

    /// `member` departed `community`: discard its rows; how many.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_member(
        &self,
        community: &str,
        member: &str,
    ) -> Result<usize, OutboxError> {
        (self.0)
            .discard_streams(StreamKind::MekTransfer, &scope(community, member), DEPARTED)
            .await
    }

    /// We left `community`: discard every row of its streams; how many.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_community(&self, community: &str) -> Result<usize, OutboxError> {
        (self.0)
            .discard_streams(StreamKind::MekTransfer, &scope(community, ""), DEPARTED)
            .await
    }
}

impl OutboxSlot {
    /// The running outbox's `mek_transfer` streams.
    pub(crate) fn mek_transfers(&self) -> Option<MekTransfers> {
        self.current.read().clone().map(MekTransfers)
    }

    pub(super) fn install_mek_transfer_sender(&self, governance: HostGovernance) {
        (self.senders).install(
            StreamKind::MekTransfer,
            Arc::new(MekTransferSender(governance)),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_payload_round_trips_its_route_and_envelope() {
        let payload = super::encode(b"route", b"sealed").unwrap();
        assert_eq!(
            super::decode(&payload),
            Some((&b"route"[..], &b"sealed"[..]))
        );
        assert_eq!(super::decode(&payload[..6]), None);
    }
}
