//! The ring hint `HeadAdvanced` (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3/§4.5): a ring watcher tells the mesh that a member record it
//! watches advanced a subkey. It carries no content, so it is
//! self-verifying: the recipient reads the writer-signed value. Its
//! signature authenticates the relaying member, as every gossip envelope
//! does, under `domains::HEAD_ADVANCED`.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, GOV_HEAD, OUTBOX, ROUTES};
use rekindle_types::domains;

use super::{check_len, check_size, put_lp, sign, verify};
use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::member_record_capnp::head_advanced;
use crate::CodecError;

/// The `M` subkeys a hint names: the beat (status transitions only), the
/// op-log head, the frontier (the writer's commit point, written after the
/// head, which a reader catches up from), the outbox and the routes
/// (`MemberRoutes`, plan E3.4 S4), relayed on every change: a route changes
/// only when Veilid replaces it, so unlike the beat it needs no transition
/// filter. The card (0), the history ad (5) and the reserved subkeys are
/// never hinted.
pub const RELAYED_SUBKEYS: [u32; 5] = [BEAT, GOV_HEAD, FRONTIER, OUTBOX, ROUTES];

/// Largest encoded hint.
pub const HEAD_ADVANCED_MAX_BYTES: usize = 512;

/// A record key at its longest text form, with room.
const MAX_RECORD_KEY: usize = 160;

/// `author`'s member record `member_record` advanced `subkey` to `seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadAdvanced {
    pub author: [u8; 32],
    pub member_record: String,
    pub subkey: u32,
    pub seq: u64,
}

impl HeadAdvanced {
    fn signing_bytes(&self, community_id: &str) -> Result<Vec<u8>, CodecError> {
        let mut out = domains::HEAD_ADVANCED.as_bytes().to_vec();
        put_lp(&mut out, community_id.as_bytes())?;
        out.extend_from_slice(&self.author);
        put_lp(&mut out, self.member_record.as_bytes())?;
        out.extend_from_slice(&self.subkey.to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        Ok(out)
    }

    /// Sign by the relaying member's pseudonym `relayer` and encode.
    ///
    /// # Errors
    /// The subkey is not relayed, or the record key or the hint is over
    /// its cap.
    pub fn encode(&self, community_id: &str, relayer: &SigningKey) -> Result<Vec<u8>, CodecError> {
        check_subkey(self.subkey)?;
        check_len(
            "member record key",
            self.member_record.len(),
            MAX_RECORD_KEY,
        )?;
        let sig = sign(relayer, &self.signing_bytes(community_id)?);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<head_advanced::Builder<'_>>();
            b.set_author(&self.author);
            b.set_member_record(&self.member_record);
            b.set_subkey(self.subkey);
            b.set_seq(self.seq);
            b.set_sig(&sig);
        }
        let out = pack(&builder)?;
        check_size("head advanced", &out, HEAD_ADVANCED_MAX_BYTES)?;
        Ok(out)
    }
}

fn check_subkey(subkey: u32) -> Result<(), CodecError> {
    if RELAYED_SUBKEYS.contains(&subkey) {
        Ok(())
    } else {
        Err(CodecError::Deserialization(format!(
            "head advanced: subkey {subkey} is not relayed"
        )))
    }
}

/// Decode a hint and verify its signature by the relaying member
/// `relayer` (the gossip envelope's sender) in `community_id`.
///
/// # Errors
/// Over the cap, malformed, a subkey that is not relayed, or a bad
/// signature.
pub fn decode_head_advanced(
    data: &[u8],
    community_id: &str,
    relayer: &[u8; 32],
) -> Result<HeadAdvanced, CodecError> {
    check_size("head advanced", data, HEAD_ADVANCED_MAX_BYTES)?;
    let reader = unpack_bounded(data, bounds::HEAD_ADVANCED_BOUND)?;
    let b = reader
        .get_root::<head_advanced::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let author: [u8; 32] = b
        .get_author()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization("head advanced: author is not 32 bytes".into()))?;
    let hint = HeadAdvanced {
        author,
        member_record: text_to_string(b.get_member_record().map_err(|e| capnp_err(&e))?)?,
        subkey: b.get_subkey(),
        seq: b.get_seq(),
    };
    check_subkey(hint.subkey)?;
    verify(
        relayer,
        &hint.signing_bytes(community_id)?,
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "head advanced",
    )?;
    Ok(hint)
}

#[cfg(test)]
#[path = "head_advanced_tests.rs"]
mod tests;
