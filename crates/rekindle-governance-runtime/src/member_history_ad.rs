//! Our member record's history ad (`M`/5) and the read of another
//! member's (plan E3.4 S7, "History ad moves to `M`/5"; ADR 0011).
//!
//! The ad names the channel ranges a member holds, so a joiner's bundle
//! and a history request can target a member that has them (E3.5: "a
//! history advertiser (a member whose `M`/5 ad names the ranges) serves
//! it"). Its community-scope part is sealed under the current
//! `KeyScope::Community` key as W11.2 sealed the registry row's ad
//! (`rekindle_codec::member_record::history_ad`), so a member removed by a
//! rotation cannot open an ad written after it.
//!
//! **Bound.** At most [`HISTORY_AD_MAX_RANGES`] ranges, the most recently
//! active channels first ([`advertised`]); the codec proves the encoded ad
//! fits [`HISTORY_AD_MAX_BYTES`](rekindle_codec::member_record::history_ad::HISTORY_AD_MAX_BYTES)
//! at that count. The registry row's ad evicted the least recently active
//! channels the same way (`rekindle-presence` `fit_history_ad`).
//!
//! **Reads.** A value read from `M(p)`/5 is accepted only from the record
//! whose descriptor owner is the `memberRecordOwner` `p`'s `MemberJoined`
//! names (`crate::member_card::read_pinned`); the reader then verifies and
//! opens it with `rekindle_sync::history::decode_member_history_ads`.

use rekindle_codec::member_record::history_ad::{encode_history_ad, HISTORY_AD_MAX_RANGES};
use rekindle_codec::member_record::RecordScope;
use rekindle_records::port::RecordPort;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::dht_layout::member_record;
use rekindle_types::presence::HistoryRange;
use rekindle_utils::random::RandomSource;

use crate::member_card::{read_pinned, write_subkey, MemberRecordRef, OwnRecord};

/// The ranges an ad of `ranges` carries: the most recently active channels
/// first (by newest lamport, ties by channel), at most
/// [`HISTORY_AD_MAX_RANGES`].
#[must_use]
pub fn advertised(mut ranges: Vec<HistoryRange>) -> Vec<HistoryRange> {
    ranges.sort_by(|a, b| {
        (b.newest_lamport.cmp(&a.newest_lamport)).then_with(|| a.channel_id.0.cmp(&b.channel_id.0))
    });
    ranges.truncate(HISTORY_AD_MAX_RANGES);
    ranges
}

/// Write our history ad (`M`/5): `ranges` as [`advertised`] picks them,
/// sealed under `key`, the current `KeyScope::Community` key.
///
/// # Errors
/// The ad does not encode, or the write did not land.
pub async fn write_history_ad(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    ranges: &[HistoryRange],
    key: &MediaEncryptionKey,
    rng: &dyn RandomSource,
) -> Result<(), String> {
    let at = RecordScope {
        community_id: own.community_id,
        member_record: own.record,
    };
    let bytes = encode_history_ad(ranges, key, at, own.pseudonym, rng)
        .map_err(|e| format!("history ad: {e}"))?;
    write_subkey(port, own, member_record::HISTORY_AD, bytes).await
}

/// A member's `M`/5 as stored, read (forced) from the record its
/// `MemberJoined` names with the owner pinned; `None` while it holds no
/// ad. The caller verifies and opens it
/// (`rekindle_sync::history::decode_member_history_ads`).
///
/// # Errors
/// The record's owner is not the one `MemberJoined` names, or the read
/// failed.
pub async fn read_history_ad(
    port: &dyn RecordPort,
    member: &MemberRecordRef<'_>,
) -> Result<Option<Vec<u8>>, String> {
    read_pinned(port, member, member_record::HISTORY_AD).await
}

#[cfg(test)]
#[path = "member_history_ad_tests.rs"]
mod tests;
