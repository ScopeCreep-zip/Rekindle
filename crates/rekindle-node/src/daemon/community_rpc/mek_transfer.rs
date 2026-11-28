//! Inbound wrapped-MEK delivery from the deterministic rotator.
//!
//! `communities-channels.md` ("Distribution paths") puts this on the
//! critical path for forward secrecy: when a member departs, the
//! elected rotator generates a new key and *delivers the wrapped MEK to
//! each member via `app_call`*. Without this handler the daemon has no
//! way to receive one — it learned MEKs only from the legacy
//! join answer (retired, plan E3.2 S8) and from the v1.0 registry MEK vault, and the vault
//! is being retired. A daemon member would have gone silently
//! undecryptable after the first rotation in any community it shares
//! with a desktop peer.
//!
//! The envelope arrives unsigned (see
//! `Caller::call_community_envelope`); authenticity comes from the
//! wrap itself, because `unwrap_mek` feeds the rotator's pseudonym
//! public key into the ECDH. A forged `sender_pseudonym` therefore
//! fails to decrypt rather than being believed — which is why the only
//! thing this handler does with the claimed sender is try it as a key.

use rekindle_codec::community::envelope::MekTransferPayload;
use rekindle_transport::payload::rpc::CallResponse;

/// Hand a wrapped MEK to the Host's one receive path
/// ([`crate::host::adapters::governance::HostGovernance::receive_mek_transfer`],
/// which the desktop shell's `MekTransfer` arm uses too).
///
/// Returns `CallResponse::Ok(ack)`, the encoded `MekTransferAck` whose
/// key-confirmation tag is keyed by the key we unwrapped (review S7m part
/// 1), which the transport layer replies with verbatim; any refusal makes
/// the rotator treat the delivery as failed, which is the correct signal
/// when we could not unwrap.
pub(crate) fn handle_mek_transfer(
    transfer: &MekTransferPayload,
    governance: Option<&crate::host::adapters::governance::HostGovernance>,
) -> CallResponse {
    let Some(governance) = governance else {
        return CallResponse::Rejected {
            reason: "locked".into(),
        };
    };
    match governance.receive_mek_transfer(transfer) {
        Ok(ack) => {
            // Keyed by the call: the key drop watch is done (plan RV2d).
            governance.spawn_key_drop_release(&transfer.community_id);
            CallResponse::Ok(ack)
        }
        Err(refusal) => CallResponse::Rejected {
            reason: refusal.reason().into(),
        },
    }
}
