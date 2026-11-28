//! Our member record's routes (`M`/6) and the joiner's read of its
//! inviter's (plan E3.2 S7c.8 item (4), the E3.4 `MemberRoutes` row of
//! D15).
//!
//! **Why the record and not the invite.** A private route blob is only
//! good while its owner keeps the route: Veilid replaces a route it finds
//! dead (`RouteChange`), and an importer's copy of a blob expires from its
//! remote-route cache after five minutes unused
//! (`veilid-core/src/routing_table/route_spec_store/mod.rs:43-44`), so a
//! blob frozen into an invite when it was minted stops reaching the
//! inviter the first time its route is replaced, and every later joiner
//! loses the pre-join hint and the bootstrap call. The invite therefore
//! names the inviter's member record and pseudonym, and the joiner reads
//! the inviter's current route from `M`/6 when it uses the invite, as a
//! Matrix invite names a server that is resolved when the invite is used
//! and a Signal group link is looked up on the server when it is opened;
//! the inviter rewrites `M`/6 on every new route (the route publisher,
//! `rekindle-node` `member_routes_loop`).
//!
//! **Owner** ([`RoutePin`], plan E3.4 S4). A device's route is read from
//! its effective record. The descriptor owner must match its membership
//! or device endorsement (`crate::member_card::read_pinned`).
//! The joiner has no merged endorsement to
//! pin its inviter's record against before its walk; it needs none: the
//! record key is the hash of the owner key and the schema
//! (`veilid-core/src/veilid_api/types/dht/derivation.rs:57-69`), and the key
//! comes from the invite's code-authenticated secrets. Either way the value
//! is accepted only under the member's pseudonym.

use rekindle_codec::member_record::{routes::decode_member_routes, MemberRoutes, RecordScope};
use rekindle_records::port::{OpenIntent, RecordPort};
use rekindle_types::dht_layout::member_record;
use rekindle_types::id::PseudonymKey;

use crate::member_card::{
    read_pinned_value, write_republished_subkey, MemberRecordRef, OwnRecord, SubkeyWriteError,
};

/// What fixes the owner of the record a route is read from.
#[derive(Debug, Clone, Copy)]
pub enum RoutePin<'a> {
    /// An effective device record, pinned to the owner from MemberJoined
    /// or its device endorsement.
    Joined(&'a str),
    /// The inviter's record named by the invite's code-authenticated
    /// secrets, whose key already fixes the owner (the joiner's pre-walk
    /// read).
    Invite,
}

/// Write our routes (`M`/6): `blob`, issued at `now_ms`.
///
/// # Errors
/// The value does not encode (an empty or oversized blob), or the write
/// did not land.
pub async fn write_routes(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    blob: &[u8],
    now_ms: u64,
) -> Result<(), SubkeyWriteError> {
    let at = RecordScope {
        community_id: own.community_id,
        member_record: own.record,
    };
    let bytes = MemberRoutes {
        general: blob.to_vec(),
        issued_at: now_ms,
    }
    .encode(at, own.pseudonym)
    .map_err(|e| SubkeyWriteError::Failed(format!("member routes: {e}")))?;
    write_republished_subkey(port, own, member_record::ROUTES, bytes).await
}

/// A member's current general route blob, read from `M(member)`/6 (forced)
/// and verified under `member`; `None` while the record holds no routes.
/// The joiner reads its inviter's this way ([`RoutePin::Invite`]), and,
/// when the inviter answers `bootstrapPartial`, those of the members its
/// walk shows (`crate::bootstrap_peers`); the Host reads every member's
/// through it ([`RoutePin::Joined`]).
///
/// # Errors
/// The record is unreadable, its owner is not the pinned one, or its
/// routes do not verify under `member` for this record.
pub async fn read_member_route(
    port: &dyn RecordPort,
    community_id: &str,
    record: &str,
    member: &PseudonymKey,
    pin: RoutePin<'_>,
) -> Result<Option<Vec<u8>>, String> {
    let read = read_member_route_at(port, community_id, record, member, pin).await?;
    Ok(read.map(|(_, blob)| blob))
}

/// [`read_member_route`] with the DHT seq of the `M`/6 value read (the
/// Host's member route cache orders by it).
///
/// # Errors
/// As [`read_member_route`].
pub async fn read_member_route_at(
    port: &dyn RecordPort,
    community_id: &str,
    record: &str,
    member: &PseudonymKey,
    pin: RoutePin<'_>,
) -> Result<Option<(u64, Vec<u8>)>, String> {
    let read = match pin {
        RoutePin::Joined(owner) => {
            let member_ref = MemberRecordRef {
                community_id,
                pseudonym: member,
                record,
                owner,
            };
            read_pinned_value(port, &member_ref, member_record::ROUTES).await?
        }
        RoutePin::Invite => {
            let lease = port
                .acquire(record, None, OpenIntent::Check)
                .await
                .map_err(|e| format!("open member record: {e}"))?;
            let read = port.get(lease, member_record::ROUTES, true).await;
            port.release(lease).await;
            read.map_err(|e| format!("read member routes: {e}"))?
        }
    };
    let Some(value) = read else {
        return Ok(None);
    };
    let at = RecordScope {
        community_id,
        member_record: record,
    };
    decode_member_routes(&value.data, at, &member.0)
        .map(|routes| Some((u64::from(value.seq), routes.general)))
        .map_err(|e| format!("routes of {record}: {e}"))
}

#[cfg(test)]
#[path = "member_routes_tests.rs"]
mod tests;
