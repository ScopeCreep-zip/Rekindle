//! Opening a session to send on (Sesame §3.3): a sender encrypts on the
//! active session with the recipient and creates one from a prekey bundle
//! only when there is none. The caller fetched the bundle after a send found
//! no session, which takes a network read; the peer's own initiation can be
//! accepted in that window (a crossing, plan E2.5h). So whether a session is
//! active is decided again here, in the transaction that would initiate: an
//! initiation never replaces a session that opened meanwhile, which the peer
//! already answers on (they would refuse ours over it, ruling Q1).

use rekindle_crypto::signal::{Outgoing, PreKeyBundle};
use rekindle_crypto::CryptoError;

use super::tx::in_transaction;
use super::{initiate_in, load, store, LocalParty, PeerAddress, RatchetError};
use crate::Db;

/// Encrypt `plaintext` for `peer` on the active session, or, when none is
/// active (or ours went unacknowledged past the wrap age, `NoSession`), start
/// one from `bundle` with it as the first message, as
/// [`super::initiate_and_record_outbound`] does. An initiation is
/// [`Outgoing::PreKey`].
///
/// # Errors
/// See [`RatchetError`].
pub async fn open_and_record_outbound(
    db: &Db,
    local: &LocalParty,
    peer: &PeerAddress,
    bundle: PreKeyBundle,
    plaintext: &[u8],
    ad_extra: &[u8],
) -> Result<Outgoing, RatchetError> {
    let (local, peer) = (local.clone(), peer.clone());
    let (plaintext, ad_extra) = (plaintext.to_vec(), ad_extra.to_vec());
    in_transaction(db, move |conn| {
        let owner = local.owner();
        let record = load(conn, &owner, &peer)?;
        match record.encrypt(&plaintext, &ad_extra, local.now_ms) {
            Ok((next, outgoing)) => {
                store(conn, &owner, &peer, &next, local.now_ms)?;
                Ok(outgoing)
            }
            Err(CryptoError::NoSession(_)) => {
                initiate_in(conn, &local, &peer, &record, &bundle, &plaintext, &ad_extra)
                    .map(Outgoing::PreKey)
            }
            Err(e) => Err(e.into()),
        }
    })
    .await
}
