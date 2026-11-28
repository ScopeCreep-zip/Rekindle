//! The one receive path of a delivered community key (`MekTransfer`, D16):
//! a rotator's or an inviter's wrapped MEK, received by either host.
//!
//! `communities-channels.md` ("Distribution paths") puts the transfer on the
//! critical path for forward secrecy: after a departure the elected rotator
//! delivers the wrapped key to each member point to point (ADR 0012, 0018).
//! Both hosts hand the payload here (plan E3.1, CB3 glue audit; ADR 0010:
//! the Host owns the node, shells are thin): the daemon's `app_call` handler
//! and the desktop shell's `ControlPayload::MekTransfer` arm.
//!
//! The envelope arrives unsigned; authenticity comes from the wrap itself,
//! because the unwrap feeds the rotator's pseudonym public key into the ECDH.
//! A forged `sender_pseudonym` therefore fails to decrypt rather than being
//! believed, which is why the claimed sender is used only as a key and to
//! refuse a departed member.

use rekindle_codec::community::envelope::MekTransferPayload;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::subscription_events::{CryptoEvent, SubscriptionEvent};

use super::HostGovernance;

/// Why a transfer was refused. The daemon replies it as the call's
/// rejection reason; the sender settles a transfer only on an ack whose
/// key-confirmation tag proves the unwrap, so any refusal makes it retry or
/// give up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MekTransferRefusal {
    /// A community we are not in: refused before any crypto runs.
    NotAMember,
    /// A community we were removed from, or a departed rotator (plan E3.2
    /// S9, R3-1/R3-3).
    Removed,
    /// No unlocked identity to unwrap with.
    Locked,
    /// The payload names no key scope we hold.
    InvalidScope,
    /// Not wrapped for us, a forged sender, or a key whose generation is
    /// not the one the transfer names.
    UnwrapFailed,
    /// The key was taken but the ack could not be encoded.
    AckNotEncoded,
}

impl MekTransferRefusal {
    /// The reason as the daemon's call rejection carries it.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotAMember => "not a member",
            Self::Removed => "removed",
            Self::Locked => "locked",
            Self::InvalidScope => "invalid key scope",
            Self::UnwrapFailed => "unwrap failed",
            Self::AckNotEncoded => "ack not encoded",
        }
    }
}

impl HostGovernance {
    /// Receive a wrapped key: refuse a community we are not in, one we were
    /// removed from and a departed rotator; unwrap it with our community
    /// pseudonym (the generation inside must be the one the transfer names);
    /// install it through [`Self::install_transferred_mek`] (convergence
    /// rule, shell keys, vault, waiting posts, outbox catch-up); tell every
    /// subscriber when it was taken (`CryptoEvent::MekRotated`); and answer
    /// the encoded `MekTransferAck` whose tag is keyed by the unwrapped key
    /// (review S7m part 1). A key the convergence rule refuses (stale or
    /// outranked) is still acked: we hold one at least as good, so the
    /// sender's retries would only repeat it.
    pub fn receive_mek_transfer(
        &self,
        transfer: &MekTransferPayload,
    ) -> Result<Vec<u8>, MekTransferRefusal> {
        let anchor = &transfer.community_id;
        if !self.ctx.communities.map().read().contains_key(anchor) {
            tracing::debug!(community = %anchor, "MEK transfer for a community we are not in — refusing");
            return Err(MekTransferRefusal::NotAMember);
        }
        let rotator = rekindle_types::id::PseudonymKey::from_hex_lossy(&transfer.sender_pseudonym);
        if !self.takes_traffic_from(anchor, &rotator) {
            tracing::debug!(community = %anchor, "MEK transfer refused: removed or departed");
            return Err(MekTransferRefusal::Removed);
        }
        let Some(secret) = self.identity_secret_impl() else {
            tracing::warn!(community = %anchor, "locked — cannot unwrap MEK transfer");
            return Err(MekTransferRefusal::Locked);
        };
        // The payload names the community key by an absent channel and a
        // channel's key by its id; anything else is not a key we hold.
        let Some(scope) = KeyScope::from_wire(transfer.channel_id.as_deref()) else {
            return Err(MekTransferRefusal::InvalidScope);
        };
        let projected = rekindle_transport::payload::rpc::MekTransferPayload {
            scope,
            generation: transfer.generation,
            rotator_pseudonym_hex: transfer.sender_pseudonym.clone(),
            wrapped_mek: transfer.wrapped_mek.clone(),
        };
        let mek = rekindle_transport::operations::mek::unwrap_mek_transfer_payload(
            &projected, &secret, anchor,
        )
        .map_err(|e| {
            // Expected whenever the transfer was not wrapped for us; at warn
            // because a legitimate rotator hitting it is a real
            // key-distribution failure.
            tracing::warn!(community = %anchor, error = %e, "MEK transfer unwrap failed");
            MekTransferRefusal::UnwrapFailed
        })?;
        let generation = mek.generation();
        if self.install_transferred_mek(anchor, scope, &mek) {
            self.ctx
                .publish_event(SubscriptionEvent::Crypto(CryptoEvent::MekRotated {
                    community: anchor.clone(),
                    channel: scope.wire_channel(),
                    generation,
                    rotator_pseudonym: None,
                }));
        }
        tracing::info!(community = %anchor, %scope, generation, "MEK accepted from rotator");
        let our_pseudonym =
            rekindle_transport::crypto::pseudonym::derive_community_pseudonym(&secret, anchor);
        let our_pseudonym_hex = hex::encode(our_pseudonym.verifying_key().to_bytes());
        rekindle_mek_rotation::encode_mek_transfer_ack(transfer, &mek, &our_pseudonym_hex).map_err(
            |e| {
                tracing::warn!(community = %anchor, error = %e, "MekTransferAck not encoded");
                MekTransferRefusal::AckNotEncoded
            },
        )
    }
}
