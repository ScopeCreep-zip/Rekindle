//! The `MekTransferAck` (review S7m part 1, research batch 11 (2)): the
//! receiver of a `MekTransfer` answers with a key-confirmation tag keyed
//! by the key it unwrapped (`rekindle_secrets::mek_ack`, as RFC 9420
//! §12.4.3's `confirmation_tag`), and the sender settles the transfer only
//! when the tag verifies under its own key of that generation, the ack
//! names this community and the joiner is the requester. A tag that
//! verifies settles the generation for that joiner whatever its transfer
//! id: a late ack of an earlier retry of the same key still counts.

use rekindle_codec::community::envelope::{
    CommunityEnvelope, ControlPayload, MekTransferAckPayload, MekTransferPayload,
};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_secrets::mek_ack::{mek_ack_tag, verify_mek_ack_tag, MekAckContext};
use rekindle_types::channel_keys::KeyScope;

use crate::error::MekRotationError;

/// What a sender expects its transfer's ack to confirm.
#[derive(Clone, Copy)]
pub struct ExpectedAck<'a> {
    pub community_id: &'a str,
    /// Our (the sender's) pseudonym, hex.
    pub inviter_hex: &'a str,
    /// The recipient's pseudonym, hex.
    pub joiner_hex: &'a str,
    pub scope: KeyScope,
    /// The key the transfer carried.
    pub mek: &'a MediaEncryptionKey,
}

/// The receiver's reply to `transfer` once it unwrapped `mek`: an encoded
/// `MekTransferAck` carrying the tag that proves it.
///
/// # Errors
/// The envelope did not encode.
pub fn encode_mek_transfer_ack(
    transfer: &MekTransferPayload,
    mek: &MediaEncryptionKey,
    joiner_hex: &str,
) -> Result<Vec<u8>, MekRotationError> {
    let scope = transfer.channel_id.clone().unwrap_or_default();
    let tag = mek_ack_tag(
        mek.as_bytes(),
        &MekAckContext {
            community_id: &transfer.community_id,
            scope: &scope,
            generation: mek.generation(),
            transfer_id: &transfer.transfer_id,
            inviter: &transfer.sender_pseudonym,
            joiner: joiner_hex,
        },
    );
    let ack = CommunityEnvelope::Control(ControlPayload::MekTransferAck(MekTransferAckPayload {
        community_id: transfer.community_id.clone(),
        channel_id: transfer.channel_id.clone(),
        generation: mek.generation(),
        requester_pseudonym: joiner_hex.to_owned(),
        transfer_id: transfer.transfer_id.clone(),
        ack_tag: tag.to_vec(),
    }));
    rekindle_codec::capnp_envelope::encode_community_envelope(&ack)
        .map_err(|e| MekRotationError::Transport(format!("encode MekTransferAck: {e}")))
}

/// Why a reply does not settle a transfer.
fn refusal(expected: &ExpectedAck<'_>, ack: &MekTransferAckPayload) -> Option<&'static str> {
    if ack.community_id != expected.community_id {
        return Some("another community");
    }
    if ack.requester_pseudonym != expected.joiner_hex {
        return Some("another requester");
    }
    if ack.generation != expected.mek.generation() {
        return Some("another generation");
    }
    if KeyScope::from_wire(ack.channel_id.as_deref()) != Some(expected.scope) {
        return Some("another key scope");
    }
    let scope = expected.scope.wire_channel().unwrap_or_default();
    let ctx = MekAckContext {
        community_id: expected.community_id,
        scope: &scope,
        generation: ack.generation,
        transfer_id: &ack.transfer_id,
        inviter: expected.inviter_hex,
        joiner: expected.joiner_hex,
    };
    (!verify_mek_ack_tag(expected.mek.as_bytes(), &ctx, &ack.ack_tag)).then_some("a bad tag")
}

/// `true` only for a `MekTransferAck` that confirms exactly `expected`:
/// the receipt that settles a `mek_transfer` row. A bare `ACK` (the
/// receiver's unwrap failed), another variant or any mismatch is `false`.
#[must_use]
pub fn is_mek_transfer_ack(expected: &ExpectedAck<'_>, reply: &[u8]) -> bool {
    let community = expected.community_id;
    let joiner = expected.joiner_hex;
    let Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransferAck(ack)))) =
        rekindle_codec::capnp_envelope::try_decode_community_envelope(reply)
    else {
        tracing::debug!(
            community,
            joiner,
            "MEK transfer reply is not a MekTransferAck"
        );
        return false;
    };
    let Some(reason) = refusal(expected, &ack) else {
        return true;
    };
    tracing::warn!(community, joiner, reason, "MekTransferAck refused");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITER: &str = "aa";
    const JOINER: &str = "bb";

    fn transfer() -> MekTransferPayload {
        MekTransferPayload {
            community_id: "c1".into(),
            channel_id: None,
            generation: 4,
            sender_pseudonym: INVITER.into(),
            wrapped_mek: vec![1],
            transfer_id: vec![9; 16],
        }
    }

    fn expected(mek: &MediaEncryptionKey) -> ExpectedAck<'_> {
        ExpectedAck {
            community_id: "c1",
            inviter_hex: INVITER,
            joiner_hex: JOINER,
            scope: KeyScope::Community,
            mek,
        }
    }

    fn reencode(ack: &[u8], edit: impl FnOnce(&mut MekTransferAckPayload)) -> Vec<u8> {
        let Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransferAck(mut p)))) =
            rekindle_codec::capnp_envelope::try_decode_community_envelope(ack)
        else {
            panic!("not an ack")
        };
        edit(&mut p);
        rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(
            ControlPayload::MekTransferAck(p),
        ))
        .unwrap()
    }

    #[test]
    fn the_joiners_ack_of_the_key_settles_and_nothing_else_does() {
        let mek = MediaEncryptionKey::from_bytes([7; 32], 4);
        let ack = encode_mek_transfer_ack(&transfer(), &mek, JOINER).unwrap();
        assert!(is_mek_transfer_ack(&expected(&mek), &ack));
        // A late ack of another retry (another id) of the same key counts.
        let other_id = encode_mek_transfer_ack(
            &MekTransferPayload {
                transfer_id: vec![1; 16],
                ..transfer()
            },
            &mek,
            JOINER,
        )
        .unwrap();
        assert!(is_mek_transfer_ack(&expected(&mek), &other_id));

        assert!(!is_mek_transfer_ack(&expected(&mek), b"ACK"));
        // Another key of the same generation (a forged or wrong unwrap).
        let forged = MediaEncryptionKey::from_bytes([8; 32], 4);
        let forged_ack = encode_mek_transfer_ack(&transfer(), &forged, JOINER).unwrap();
        assert!(!is_mek_transfer_ack(&expected(&mek), &forged_ack));
        // Another party answering for the joiner.
        let other = encode_mek_transfer_ack(&transfer(), &mek, "cc").unwrap();
        assert!(!is_mek_transfer_ack(&expected(&mek), &other));
        // Every field the ack names is checked.
        for edit in [
            (|p: &mut MekTransferAckPayload| p.community_id = "c2".into())
                as fn(&mut MekTransferAckPayload),
            |p| p.requester_pseudonym = "cc".into(),
            |p| p.generation = 5,
            |p| p.channel_id = Some("ch".into()),
            |p| p.ack_tag.clear(),
            |p| p.transfer_id = vec![2; 16],
        ] {
            assert!(!is_mek_transfer_ack(&expected(&mek), &reencode(&ack, edit)));
        }
    }
}
