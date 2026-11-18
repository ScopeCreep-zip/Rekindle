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
//! **Owner.** The joiner has no `MemberJoined` to pin `M`'s owner against
//! before its walk; it needs none: the record key is the hash of the owner
//! key and the schema (`veilid-core/src/veilid_api/types/dht/derivation.rs:57-69`),
//! and the key comes from the invite's code-authenticated secrets. The
//! value is accepted only under the pseudonym the same secrets name.

use rekindle_codec::member_record::{routes::decode_member_routes, MemberRoutes, RecordScope};
use rekindle_records::port::{OpenIntent, RecordPort};
use rekindle_types::dht_layout::member_record;
use rekindle_types::id::PseudonymKey;

use crate::member_card::{write_subkey, OwnRecord};

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
) -> Result<(), String> {
    let at = RecordScope {
        community_id: own.community_id,
        member_record: own.record,
    };
    let bytes = MemberRoutes {
        general: blob.to_vec(),
        issued_at: now_ms,
    }
    .encode(at, own.pseudonym)
    .map_err(|e| format!("member routes: {e}"))?;
    write_subkey(port, own, member_record::ROUTES, bytes).await
}

/// A member's current general route blob, read from `M(member)`/6 and
/// verified under `member`; `None` while the record holds no routes. The
/// joiner reads its inviter's this way, and, when the inviter answers
/// `bootstrapPartial`, those of the members its walk shows
/// (`crate::bootstrap_peers`).
///
/// # Errors
/// The record is unreadable, or its routes do not verify under `member`
/// for this record.
pub async fn read_member_route(
    port: &dyn RecordPort,
    community_id: &str,
    record: &str,
    member: &PseudonymKey,
) -> Result<Option<Vec<u8>>, String> {
    let lease = port
        .acquire(record, None, OpenIntent::Check)
        .await
        .map_err(|e| format!("open member record: {e}"))?;
    let read = port.get(lease, member_record::ROUTES, true).await;
    port.release(lease).await;
    let Some(value) = read.map_err(|e| format!("read member routes: {e}"))? else {
        return Ok(None);
    };
    let at = RecordScope {
        community_id,
        member_record: record,
    };
    decode_member_routes(&value.data, at, &member.0)
        .map(|routes| Some(routes.general))
        .map_err(|e| format!("routes of {record}: {e}"))
}

#[cfg(test)]
#[path = "member_routes_tests.rs"]
mod tests;
