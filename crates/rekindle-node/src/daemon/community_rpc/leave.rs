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
    let community_short = &notif.governance_key[..16.min(notif.governance_key.len())];
    let member_short = &notif.leaving_pseudonym_hex[..16.min(notif.leaving_pseudonym_hex.len())];

    // Only act for communities we are actually in. Without this an
    // unsolicited notification would queue rotation work for a
    // community we have no key in, which the worker would then have to
    // discover and discard.
    let is_member = session
        .read()
        .as_ref()
        .is_some_and(|s| s.community(&notif.governance_key).is_some());
    if !is_member {
        tracing::debug!(
            community = %community_short,
            "leave notification for a community we are not in — ignoring"
        );
        return CallResponse::Ack;
    }

    tracing::info!(
        community = %community_short,
        member = %member_short,
        "member departed — queueing MEK rotation"
    );

    let request = MekRotationRequest {
        community_id: notif.governance_key.clone(),
        departed_pseudonym_hex: notif.leaving_pseudonym_hex.clone(),
    };
    if mek_rotation_tx.send(request).is_err() {
        tracing::warn!(
            community = %community_short,
            "MEK rotation worker is gone — departure did not rotate the key"
        );
    }

    CallResponse::Ack
}

#[cfg(test)]
mod tests {
    use super::handle_leave;
    use parking_lot::RwLock;
    use rekindle_transport::payload::rpc::CommunityLeaveNotification;
    use rekindle_transport::session::{CommunityMembership, Session, SessionIdentity};

    const GOV: &str = "VLD0:gov:key";
    const DEPARTED: &str = "ff00";

    fn identity() -> SessionIdentity {
        SessionIdentity {
            public_key_hex: "ab".repeat(32),
            display_name: "tester".into(),
            profile_dht_key: String::new(),
            mailbox_dht_key: String::new(),
            friend_list_dht_key: String::new(),
            friend_inbox_key: String::new(),
            friend_inbox_keypair_hex: String::new(),
            profile_keypair_bytes: None,
            friend_list_keypair_bytes: None,
        }
    }

    /// A *non-operator* membership on purpose: under v2.0 the response
    /// to a departure must not depend on being privileged, which the
    /// v1.0 handler gated on.
    fn membership() -> CommunityMembership {
        CommunityMembership {
            governance_key: GOV.into(),
            pseudonym_key: "cd".repeat(32),
            display_name: "tester".into(),
            role_ids: Vec::new(),
            registry_key: "VLD0:reg:key".into(),
            slot_index: 3,
            community_name: "test".into(),
            slot_seed: None,
            channel_record_keys: std::collections::HashMap::new(),
            community_mailbox_key: String::new(),
            join_inbox_key: String::new(),
            is_operator: false,
            governance_keypair_label: None,
            segment_index: Some(0),
            lamport_counter: 0,
            mek_generation: 1,
        }
    }

    fn session_with_community() -> RwLock<Option<Session>> {
        let mut session = Session::new(identity());
        session.join_community(membership());
        RwLock::new(Some(session))
    }

    fn notification() -> CommunityLeaveNotification {
        CommunityLeaveNotification {
            governance_key: GOV.into(),
            leaving_pseudonym_hex: DEPARTED.into(),
        }
    }

    /// The whole v2.0 contract for this handler: a departure queues a
    /// rotation and does nothing else. No operator check, no member
    /// index rewrite, no MEK vault write.
    #[test]
    fn departure_queues_a_rotation_for_a_plain_member() {
        let (tx, mut rx) = crate::daemon::mek_rotation::channel();
        handle_leave(&notification(), &session_with_community(), &tx);

        let queued = rx.try_recv().expect("a rotation should have been queued");
        assert_eq!(queued.community_id, GOV);
        assert_eq!(queued.departed_pseudonym_hex, DEPARTED);
        assert!(rx.try_recv().is_err(), "exactly one request");
    }

    /// An unsolicited notification for a community we never joined must
    /// not create work. Anyone who can reach our route can send one.
    #[test]
    fn departure_for_an_unknown_community_is_ignored() {
        let (tx, mut rx) = crate::daemon::mek_rotation::channel();
        let mut notif = notification();
        notif.governance_key = "VLD0:some:other".into();

        handle_leave(&notif, &session_with_community(), &tx);
        assert!(rx.try_recv().is_err(), "must not queue for a stranger");
    }

    /// No session at all (daemon locked) is the same answer.
    #[test]
    fn departure_with_no_session_is_ignored() {
        let (tx, mut rx) = crate::daemon::mek_rotation::channel();
        handle_leave(&notification(), &RwLock::new(None), &tx);
        assert!(rx.try_recv().is_err());
    }

    /// A dead worker must not take the handler down with it — the
    /// leaving peer is already gone and cannot act on a failure.
    #[test]
    fn a_dropped_worker_does_not_panic_the_handler() {
        let (tx, rx) = crate::daemon::mek_rotation::channel();
        drop(rx);
        handle_leave(&notification(), &session_with_community(), &tx);
    }
}
