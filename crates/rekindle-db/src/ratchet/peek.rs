//! What a session with a peer allows, read without a step.

use crate::Db;

use super::{in_transaction, load, PeerAddress, RatchetError};

/// Whether our active session with `peer` is established: something
/// decrypted under it. No session, or only our unanswered init, is not.
///
/// # Errors
/// The database refused.
pub async fn is_established(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
) -> Result<bool, RatchetError> {
    let (owner, peer) = (*owner, peer.clone());
    in_transaction(db, move |conn| {
        Ok(load(conn, &owner, &peer)?.is_established())
    })
    .await
}

/// Whether we hold an active session with `peer`, one a frame to them can
/// be encrypted under (our unanswered init counts). With none, only a kind
/// that may open a session can be sent (live A/B finding 69).
///
/// # Errors
/// The database refused.
pub async fn has_session(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
) -> Result<bool, RatchetError> {
    let (owner, peer) = (*owner, peer.clone());
    in_transaction(db, move |conn| Ok(load(conn, &owner, &peer)?.has_active())).await
}

/// Whether a session we hold with `peer` has `base_key`: a PreKeyInit with
/// it is a message on that session (a retransmission, or one the peer sent
/// before we answered, plan E2.5h), judged as a ratchet message of its kind,
/// not as a session opening.
///
/// # Errors
/// The database refused.
pub async fn is_known_init(
    db: &Db,
    owner: &[u8; 32],
    peer: &PeerAddress,
    base_key: [u8; 32],
) -> Result<bool, RatchetError> {
    let (owner, peer) = (*owner, peer.clone());
    in_transaction(db, move |conn| {
        Ok(load(conn, &owner, &peer)?.has_base_key(&base_key))
    })
    .await
}
