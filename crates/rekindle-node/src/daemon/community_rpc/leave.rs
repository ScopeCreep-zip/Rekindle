//! Departure notification from a member who is leaving.
//!
//! **Under v2.0 this handler does not remove anybody.**
//! `communities-governance.md`: *"Leaving is unilateral: stop
//! heartbeat, zero own registry slot, close records… No coordinator
//! approval."* The leaver owns its own SMPL subkey, so it retires
//! itself; nothing here can do that on its behalf and nothing here
//! needs to.
//!
//! What this notification is actually good for is *speed*. A departure
//! is otherwise noticed when presence goes stale, and the MEK the
//! departing member still holds stays valid until then. Telling peers
//! directly starts the rotation immediately.
//!
//! The v1.0 handler did something else entirely: it gated on
//! `is_operator`, rewrote the registry member index, edited the MEK
//! vault, and rekeyed every channel by itself. Three things wrong with
//! that under flat governance — it made one peer privileged, it wrote
//! owner-subkey structures that `o_cnt: 0` has no writer for, and it
//! bypassed the deterministic rotator so two operators receiving the
//! same notification would both rekey and fight over the generation.
//!
//! Now every recipient does the same thing: queue a rotation request.
//! `rotate_text_mek_for_departure` computes
//! `blake3(departed || own_pseudonym)` and only the lowest hash
//! actually rotates, so N recipients still produce one rotation.

use parking_lot::RwLock;

use rekindle_transport::payload::rpc::{CallResponse, CommunityLeaveNotification};

use crate::daemon::mek_rotation::{MekRotationRequest, MekRotationSender};

/// Accept a departure notification and start the forward-secrecy
/// rotation.
///
/// Returns `Ack` in every case: this is a best-effort courtesy from a
/// peer that is on its way out, and there is nothing it could usefully
/// do with a failure.
pub(crate) fn handle_leave(
    notif: &CommunityLeaveNotification,
    session: &RwLock<Option<rekindle_transport::Session>>,
    mek_rotation_tx: &MekRotationSender,
) -> CallResponse {
    let community = &notif.anchor;
    let member = &notif.leaving_pseudonym_hex;

    // Only act for communities we are actually in. Without this an
    // unsolicited notification would queue rotation work for a
    // community we have no key in, which the worker would then have to
    // discover and discard.
    let is_member = session
        .read()
        .as_ref()
        .is_some_and(|s| s.community(&notif.anchor).is_some());
    if !is_member {
        tracing::debug!(
            community = %community,
            "leave notification for a community we are not in — ignoring"
        );
        return CallResponse::Ack;
    }

    tracing::info!(
        community = %community,
        member = %member,
        "member departed — queueing MEK rotation"
    );

    let request =
        MekRotationRequest::departure(notif.anchor.clone(), notif.leaving_pseudonym_hex.clone());
    if mek_rotation_tx.send(request).is_err() {
        tracing::warn!(
            community = %community,
            "MEK rotation worker is gone — departure did not rotate the key"
        );
    }

    CallResponse::Ack
}

#[cfg(test)]
#[path = "leave_tests.rs"]
mod tests;
