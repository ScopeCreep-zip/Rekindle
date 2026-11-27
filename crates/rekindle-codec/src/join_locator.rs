//! A join inbox carries only a sealed locator. Readers still authenticate
//! and admit the referenced MemberJoined through governance.

use rekindle_secrets::{derive::pseudonym_to_x25519, ed25519_dalek, hpke};
use rekindle_types::{
    dht_layout::join_inbox::MAX_ENTRY_LEN, domains, governance::OpRef, key_format,
};
use zeroize::Zeroizing;

use crate::{
    capnp_codec::{bounds::Bound, capnp_err, pack, unpack_bounded},
    governance_op::{read_op_ref, write_op_ref},
    governance_op_capnp::op_ref,
    CodecError,
};

const VERSION: u8 = 1;
const HEADER_LEN: usize = 1 + hpke::ENC_LEN;
const MAX_PLAINTEXT: usize = MAX_ENTRY_LEN - HEADER_LEN - hpke::TAG_LEN;

/// The member-record locator is the author record in the op reference.
/// A reader verifies that the fetched MemberJoined names this same record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinLocator {
    pub joined_op: OpRef,
}

impl JoinLocator {
    /// Seal the locator to the original invite minter. The invite id binds
    /// the carrier to one invite even when the same recipient minted both.
    ///
    /// # Errors
    /// Invalid recipient or locator, or an encoding or encryption failure.
    pub fn seal(&self, recipient: &[u8; 32], invite_id: &[u8; 16]) -> Result<Vec<u8>, CodecError> {
        key_format::record_key(&self.joined_op.author_record)
            .map_err(|e| CodecError::Serialization(e.to_string()))?;
        let recipient = ed25519_dalek::VerifyingKey::from_bytes(recipient)
            .map_err(|e| CodecError::Verification(e.to_string()))?;
        let mut message = capnp::message::Builder::new_default();
        write_op_ref(message.init_root::<op_ref::Builder<'_>>(), &self.joined_op);
        let plaintext = Zeroizing::new(pack(&message));
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(CodecError::Serialization(
                "join locator exceeds its bound".into(),
            ));
        }
        let sealed = hpke::seal_base(
            &recipient.to_montgomery().to_bytes(),
            domains::JOIN_LOCATOR,
            invite_id,
            &plaintext,
        )
        .map_err(|e| CodecError::Verification(e.to_string()))?;
        let mut bytes = Vec::with_capacity(HEADER_LEN + sealed.ciphertext.len());
        bytes.push(VERSION);
        bytes.extend_from_slice(&sealed.enc);
        bytes.extend_from_slice(&sealed.ciphertext);
        Ok(bytes)
    }

    /// Read one sealed locator, with the wire and traversal limits applied
    /// before interpreting its fields.
    ///
    /// # Errors
    /// An invalid wire value, invite id, recipient or locator.
    pub fn open(
        bytes: &[u8],
        recipient: &ed25519_dalek::SigningKey,
        invite_id: &[u8; 16],
    ) -> Result<Self, CodecError> {
        if !(HEADER_LEN + hpke::TAG_LEN..=MAX_ENTRY_LEN).contains(&bytes.len())
            || bytes[0] != VERSION
        {
            return Err(CodecError::Deserialization(
                "invalid join locator carrier".into(),
            ));
        }
        let secret = Zeroizing::new(pseudonym_to_x25519(recipient).to_bytes());
        let plaintext = hpke::open_base(
            &secret,
            domains::JOIN_LOCATOR,
            invite_id,
            &bytes[1..HEADER_LEN],
            &bytes[HEADER_LEN..],
        )
        .map_err(|e| CodecError::Verification(e.to_string()))?;
        let message = unpack_bounded(&plaintext, Bound::packed(MAX_PLAINTEXT))?;
        let joined_op = read_op_ref(
            message
                .get_root::<op_ref::Reader<'_>>()
                .map_err(|e| capnp_err(&e))?,
        )?;
        key_format::record_key(&joined_op.author_record)
            .map_err(|e| CodecError::Deserialization(e.to_string()))?;
        Ok(Self { joined_op })
    }
}

#[cfg(test)]
#[path = "join_locator_tests.rs"]
mod tests;
