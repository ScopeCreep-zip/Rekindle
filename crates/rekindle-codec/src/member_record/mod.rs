//! The subkey formats of a member's own DFLT(8) `MemberRecord` `M` (ADR
//! 0011, plan E3.2, the D15 subkey map; `schemas/member_record.capnp`):
//! subkey 0 the [`card`], 1 the [`beat`] and 5 the [`history_ad`]. The
//! self-sovereign join writes all three; E3.3 and E3.4 extend the schema.
//!
//! Every subkey is signed by the member's community pseudonym under its
//! own domain, bound to the community and the record it is written in, so
//! a value copied into another record or community does not verify. The
//! reader supplies the pseudonym it expects (from the op DAG); a record
//! never names its own signer. Signing and the AEAD go through
//! `rekindle-secrets`.

pub mod beat;
pub mod card;
pub mod history_ad;
#[cfg(test)]
mod tests;

pub use beat::{PresenceBeat, SealedExtras, PRESENCE_BEAT_MAX_BYTES};
pub use card::{MemberCard, SignedMemberCard, MEMBER_CARD_MAX_BYTES};
pub use history_ad::{HISTORY_AD_MAX_BYTES, HISTORY_AD_MAX_RANGES};

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::member_record::MemberRecordAad;

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::CodecError;

/// The record a subkey value is written in.
#[derive(Debug, Clone, Copy)]
pub struct RecordScope<'a> {
    pub community_id: &'a str,
    /// The member record's key.
    pub member_record: &'a str,
}

impl<'a> RecordScope<'a> {
    pub(crate) fn aad(self) -> MemberRecordAad<'a> {
        MemberRecordAad {
            community_id: self.community_id,
            member_record: self.member_record,
        }
    }

    /// `domain ‖ community ‖ memberRecord`, each length-prefixed after the
    /// domain, the start of every subkey's signing bytes.
    pub(crate) fn signing_prefix(self, domain: &str) -> Vec<u8> {
        let mut out = domain.as_bytes().to_vec();
        put_lp(&mut out, self.community_id.as_bytes());
        put_lp(&mut out, self.member_record.as_bytes());
        out
    }
}

/// Append `u32be(len) ‖ bytes`.
pub(crate) fn put_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// Refuse a field longer than its bound.
pub(crate) fn check_len(what: &str, len: usize, max: usize) -> Result<(), CodecError> {
    if len > max {
        return Err(CodecError::Serialization(format!(
            "{what} is {len} bytes, the bound is {max}"
        )));
    }
    Ok(())
}

/// Refuse an encoded value over its subkey's bound, on both sides.
pub(crate) fn check_size(what: &str, data: &[u8], max: usize) -> Result<(), CodecError> {
    if data.len() > max {
        return Err(CodecError::Deserialization(format!(
            "{what} is {} bytes, the bound is {max}",
            data.len()
        )));
    }
    Ok(())
}

pub(crate) fn sign(key: &SigningKey, bytes: &[u8]) -> [u8; 64] {
    rekindle_secrets::derive::sign_with_pseudonym(key, bytes)
}

/// Verify the pseudonym's `sig` over `bytes`; `what` names the value.
pub(crate) fn verify(
    pseudonym: &[u8; 32],
    bytes: &[u8],
    sig: &[u8],
    what: &str,
) -> Result<(), CodecError> {
    let sig: [u8; 64] = sig
        .try_into()
        .map_err(|_| CodecError::Verification(format!("{what} signature length")))?;
    rekindle_secrets::derive::verify_pseudonym_signature(pseudonym, bytes, &sig)
        .map_err(|e| CodecError::Verification(format!("{what}: {e}")))
}

/// A text field, `None` when absent or empty.
pub(crate) fn opt_text(
    has: bool,
    get: Result<capnp::text::Reader<'_>, capnp::Error>,
) -> Result<Option<String>, CodecError> {
    if !has {
        return Ok(None);
    }
    let s = text_to_string(get.map_err(|e| capnp_err(&e))?)?;
    Ok((!s.is_empty()).then_some(s))
}
