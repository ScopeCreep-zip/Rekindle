//! The ring hint `HeadAdvanced` (plan E3.2 S7, `evidence/slot-integrity.md`
//! §4.3/§4.5): a ring watcher tells the mesh that a member record it
//! watches advanced a subkey. It carries no content, so it is
//! self-verifying: the recipient reads the writer-signed value. Its
//! signature authenticates the relaying member, as every gossip envelope
//! does, under `domains::HEAD_ADVANCED`.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::{BEAT, FRONTIER, GOV_HEAD, OUTBOX};
use rekindle_types::domains;

use super::{check_len, check_size, put_lp, sign, verify};
use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::member_record_capnp::head_advanced;
use crate::CodecError;

/// The `M` subkeys a hint names: the beat (status transitions only), the
/// op-log head, the frontier (the writer's commit point, written after the
/// head, which a reader catches up from) and the outbox. Subkey 6
/// (`MemberRoutes`) is reserved until E3.4.
pub const RELAYED_SUBKEYS: [u32; 4] = [BEAT, GOV_HEAD, FRONTIER, OUTBOX];

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
    fn signing_bytes(&self, community_id: &str) -> Vec<u8> {
        let mut out = domains::HEAD_ADVANCED.as_bytes().to_vec();
        put_lp(&mut out, community_id.as_bytes());
        out.extend_from_slice(&self.author);
        put_lp(&mut out, self.member_record.as_bytes());
        out.extend_from_slice(&self.subkey.to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out
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
        let sig = sign(relayer, &self.signing_bytes(community_id));
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<head_advanced::Builder<'_>>();
            b.set_author(&self.author);
            b.set_member_record(&self.member_record);
            b.set_subkey(self.subkey);
            b.set_seq(self.seq);
            b.set_sig(&sig);
        }
        let out = pack(&builder);
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
        &hint.signing_bytes(community_id),
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "head advanced",
    )?;
    Ok(hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(subkey: u32) -> HeadAdvanced {
        HeadAdvanced {
            author: [7; 32],
            member_record: format!("VLD0:{}:{}", "A".repeat(43), "b".repeat(43)),
            subkey,
            seq: u64::MAX,
        }
    }

    #[test]
    fn a_hint_round_trips_under_its_relayer_and_fits_its_cap() {
        let relayer = SigningKey::from_bytes(&[3; 32]);
        let bytes = hint(GOV_HEAD).encode("c", &relayer).unwrap();
        assert!(bytes.len() <= HEAD_ADVANCED_MAX_BYTES);
        let public = relayer.verifying_key().to_bytes();
        assert_eq!(
            decode_head_advanced(&bytes, "c", &public).unwrap(),
            hint(GOV_HEAD)
        );
    }

    /// The frontier (3), the writer's commit point, is a hinted subkey.
    #[test]
    fn a_frontier_hint_round_trips() {
        let relayer = SigningKey::from_bytes(&[3; 32]);
        let bytes = hint(FRONTIER).encode("c", &relayer).unwrap();
        let public = relayer.verifying_key().to_bytes();
        assert_eq!(
            decode_head_advanced(&bytes, "c", &public).unwrap(),
            hint(FRONTIER)
        );
    }

    #[test]
    fn another_relayer_or_community_fails_and_subkey_6_is_reserved() {
        let relayer = SigningKey::from_bytes(&[3; 32]);
        let bytes = hint(OUTBOX).encode("c", &relayer).unwrap();
        let other = SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes();
        assert!(decode_head_advanced(&bytes, "c", &other).is_err());
        let public = relayer.verifying_key().to_bytes();
        assert!(decode_head_advanced(&bytes, "d", &public).is_err());
        assert!(hint(6).encode("c", &relayer).is_err());
    }
}
