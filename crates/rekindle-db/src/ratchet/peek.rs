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
