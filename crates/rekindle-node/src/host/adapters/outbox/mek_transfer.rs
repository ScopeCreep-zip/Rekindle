//! The `mek_transfer` stream (plan E3.2 S7l, D16, D22): a community key
//! sealed to one member is a row of the stream of (community, member),
//! its payload the route the member gave and the `MekTransfer` envelope
//! already sealed to its pseudonym (never the key itself). Each attempt
//! `app_call`s the member's current route (the one its presence carries,
//! else the one it gave), and only its `MekTransferAck` of exactly that
//! key settles the row `delivered`; anything else is tried again until
//! the deadline. A member who is no longer a key reader settles its row
//! unsent, and the member's departure, or ours, discards the stream.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind,
};
use rekindle_types::channel_keys::KeyScope;

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::adapters::governance::HostGovernance;

/// The reason a row whose member departed settles with.
pub const DEPARTED: &str = "departed";

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

/// The scope and generation the sealed envelope carries.
fn sealed_key(sealed: &[u8]) -> Option<(KeyScope, u64)> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(sealed) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransfer(p)))) => {
            Some((KeyScope::from_wire(p.channel_id.as_deref())?, p.generation))
        }
        _ => None,
    }
}

struct MekTransferSender(HostGovernance);

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
        let Some((key_scope, generation)) = sealed_key(sealed) else {
            return AttemptOutcome::Wedged("the row holds no MekTransfer".into());
        };
        if !self.0.member_reads_keys(community, member) {
            return AttemptOutcome::Superseded(DEPARTED.to_owned());
        }
        let route = self
            .0
            .member_route(community, member)
            .unwrap_or_else(|| route.to_vec());
        match self.0.call_member(&route, sealed.to_vec()).await {
            Ok(reply)
                if rekindle_mek_rotation::is_mek_transfer_ack(
                    community, member, key_scope, generation, &reply,
                ) =>
            {
                tracing::info!(
                    community,
                    member,
                    generation,
                    "community key delivered (D16)"
                );
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
    /// to `route` (or the member's newer one).
    ///
    /// # Errors
    /// The storage failed, `client_id` is already queued, or the route is
    /// too long to frame.
    pub async fn queue(
        &self,
        community: &str,
        member: &str,
        client_id: &str,
        route: &[u8],
        sealed: &[u8],
    ) -> Result<(), OutboxError> {
        let payload = encode(route, sealed)
            .ok_or_else(|| OutboxError::Storage("key transfer route too long".into()))?;
        let entry = Entry {
            kind: StreamKind::MekTransfer,
            scope: scope(community, member),
            client_id: client_id.to_owned(),
            payload,
            deadline: Deadline::Horizon,
        };
        self.0.enqueue(entry, None).await.map(drop)
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
