//! `M`/5, the history ad (≤ 4 KiB), keeping the W11.2 sealing: the
//! ranges a member holds are sealed under the community text key of the
//! current `KeyScope::Community` generation, AAD = community ‖ member
//! record, and the pseudonym signs `rekindle-history-ad-v1 ‖ community ‖
//! memberRecord ‖ mekGeneration ‖ sealed`. A member removed by a rotation
//! keeps the record key but cannot open an ad sealed after it.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_secrets::member_record::{open_member_record_field, seal_member_record_field};
use rekindle_types::domains;
use rekindle_types::id::ChannelId;
use rekindle_types::presence::HistoryRange;
use rekindle_utils::random::RandomSource;

use super::{check_len, check_size, put_lp, sign, verify, RecordScope};
use crate::capnp_codec::{capnp_err, pack, unpack_bounded};
use crate::member_record_capnp::{history_ad_record, history_ranges};
use crate::CodecError;

/// The bound on an encoded history ad, proven at [`HISTORY_AD_MAX_RANGES`].
pub const HISTORY_AD_MAX_BYTES: usize = 4 * 1024;
/// The most ranges one ad carries; the writer picks which.
pub const HISTORY_AD_MAX_RANGES: usize = 64;

const MAX_WORDS: usize = 1024;

fn signing_bytes(at: RecordScope<'_>, mek_generation: u64, sealed: &[u8]) -> Vec<u8> {
    let mut out = at.signing_prefix(domains::HISTORY_AD);
    out.extend_from_slice(&mek_generation.to_be_bytes());
    put_lp(&mut out, sealed);
    out
}

fn encode_ranges(ranges: &[HistoryRange]) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let root = builder.init_root::<history_ranges::Builder<'_>>();
        let count = u32::try_from(ranges.len()).unwrap_or(u32::MAX);
        let mut list = root.init_ranges(count);
        for (i, range) in (0u32..).zip(ranges) {
            let mut r = list.reborrow().get(i);
            r.set_channel_id(&range.channel_id.0);
            r.set_oldest_lamport(range.oldest_lamport);
            r.set_newest_lamport(range.newest_lamport);
        }
    }
    pack(&builder)
}

fn decode_ranges(data: &[u8]) -> Result<Vec<HistoryRange>, CodecError> {
    let reader = unpack_bounded(data, MAX_WORDS)?;
    let list = reader
        .get_root::<history_ranges::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?
        .get_ranges()
        .map_err(|e| capnp_err(&e))?;
    check_len("history ranges", list.len() as usize, HISTORY_AD_MAX_RANGES)
        .map_err(|e| CodecError::Deserialization(e.to_string()))?;
    list.iter()
        .map(|r| {
            let id: [u8; 16] = r
                .get_channel_id()
                .map_err(|e| capnp_err(&e))?
                .try_into()
                .map_err(|_| CodecError::Deserialization("history range channel id".into()))?;
            Ok(HistoryRange {
                channel_id: ChannelId(id),
                oldest_lamport: r.get_oldest_lamport(),
                newest_lamport: r.get_newest_lamport(),
            })
        })
        .collect()
}

/// Seal `ranges` under `key` (the current `KeyScope::Community` key), sign
/// and encode the ad for the record `at`.
pub fn encode_history_ad(
    ranges: &[HistoryRange],
    key: &MediaEncryptionKey,
    at: RecordScope<'_>,
    pseudonym: &SigningKey,
    rng: &dyn RandomSource,
) -> Result<Vec<u8>, CodecError> {
    check_len("history ranges", ranges.len(), HISTORY_AD_MAX_RANGES)?;
    let sealed = seal_member_record_field(key, at.aad(), &encode_ranges(ranges), rng)
        .map_err(|e| CodecError::Serialization(e.to_string()))?;
    let generation = key.generation();
    let sig = sign(pseudonym, &signing_bytes(at, generation, &sealed));
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<history_ad_record::Builder<'_>>();
        root.set_mek_generation(generation);
        root.set_sealed(&sealed);
        root.set_sig(&sig);
    }
    let out = pack(&builder);
    check_size("history ad", &out, HISTORY_AD_MAX_BYTES)?;
    Ok(out)
}

/// Decode `M`/5, verify its signature by `pseudonym` for the record `at`
/// and open it with `key`, which must be of the ad's generation (a member
/// without the current key reads no ads).
pub fn decode_history_ad(
    data: &[u8],
    key: &MediaEncryptionKey,
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<Vec<HistoryRange>, CodecError> {
    check_size("history ad", data, HISTORY_AD_MAX_BYTES)?;
    let reader = unpack_bounded(data, MAX_WORDS)?;
    let root = reader
        .get_root::<history_ad_record::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let generation = root.get_mek_generation();
    let sealed = root.get_sealed().map_err(|e| capnp_err(&e))?;
    verify(
        pseudonym,
        &signing_bytes(at, generation, sealed),
        root.get_sig().map_err(|e| capnp_err(&e))?,
        "history ad",
    )?;
    if key.generation() != generation {
        return Err(CodecError::Verification(format!(
            "history ad sealed at generation {generation}, key is {}",
            key.generation()
        )));
    }
    let plaintext = open_member_record_field(key, at.aad(), sealed)
        .map_err(|e| CodecError::Verification(e.to_string()))?;
    decode_ranges(&plaintext)
}
