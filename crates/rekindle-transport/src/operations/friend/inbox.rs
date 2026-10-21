//! A friend invite's request inbox (plan E2.5, D17): the single-use
//! capability `SMPL(o_cnt: 1, [oneTimeWriter])`
//! (`rekindle_types::dht_layout::request_inbox`). The inviter creates it
//! with a fresh one-time writer whose keypair the invite carries; the
//! requester writes its sealed `FriendRequest` frame into the writer's
//! subkey; the inviter watches it and opens what lands through the same
//! admission as any 1:1 frame. An invite is delivered only this way: it
//! carries no route, so there is no `app_call` path to keep beside it.
//!
//! Exactly one requester can write: Veilid refuses a value signed by any
//! other writer (`veilid-core/src/storage_manager/schema.rs`
//! `check_subkey_value_data`, "wrong writer"). A second redeemer holding
//! the same leaked link is refused by the invite's state
//! (`outgoing_invites::admits_request`), and the inbox is dropped once the
//! invite leaves `pending`.

use rekindle_codec::capnp_codec::direct::RequestAuth;
use rekindle_codec::message::envelope::{InboxCapability, InviteBlob};
use rekindle_types::dht_layout::request_inbox;
use zeroize::Zeroizing;

use super::error::FriendError;
use super::host::FriendHost;
use super::state;
use crate::operations::dm;

fn records(
    host: &dyn FriendHost,
) -> Result<std::sync::Arc<dyn rekindle_records::port::RecordPort>, FriendError> {
    host.node()?
        .require_records()
        .map_err(|e| FriendError::NotReady(format!("records: {e}")))
}

/// Create an invite's inbox with a fresh one-time writer, opened on the
/// network by the owner subkey's write, held and watched
/// by the Host with its owner keypair in the vault; the capability the
/// invite carries.
///
/// # Errors
/// The record could not be created or its owner keypair kept.
pub(crate) async fn create(host: &dyn FriendHost) -> Result<InboxCapability, FriendError> {
    let mut secret = Zeroizing::new([0u8; 32]);
    host.rng().fill(secret.as_mut_slice());
    let signing = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret);
    let writer = rekindle_crypto::keypair_text::keypair_text_from_signing(&signing);
    let schema = rekindle_records::schema::request_inbox(&signing.verifying_key().to_bytes());
    let pool = records(host)?;
    let (lease, record_key, owner) = pool
        .create(&schema, None)
        .await
        .map_err(|e| FriendError::Failed(format!("request inbox not created: {e}")))?;
    let owner = Zeroizing::new(owner);
    // The owner subkey's one write puts the record on the network.
    let opened = pool
        .set(
            lease,
            request_inbox::OWNER,
            request_inbox::OPENED.to_vec(),
            None,
        )
        .await
        .and_then(|outcome| outcome.require_stored(request_inbox::OWNER));
    if let Err(e) = opened {
        pool.release(lease).await;
        return Err(FriendError::Failed(format!(
            "request inbox not opened: {e}"
        )));
    }
    host.keep_request_inbox(&record_key, &owner, lease).await?;
    Ok(InboxCapability { record_key, writer })
}

/// Redeem `blob`: write our sealed `FriendRequest`, admitted by
/// `invite_id`, into its inbox as the one-time writer. A retry writes the
/// same subkey again under the same writer.
///
/// # Errors
/// Our profile or route is not published yet, the frame cannot be sealed,
/// or the write did not land.
pub(crate) async fn redeem(
    host: &dyn FriendHost,
    blob: &InviteBlob,
    invite_id: String,
    message: &str,
) -> Result<(), FriendError> {
    let request = super::request::build(host, message, RequestAuth::Invite(invite_id)).await?;
    let frame = dm::reply_frame(host, &blob.public_key, &request).await?;
    if frame.len() > request_inbox::MAX_SUBKEY_VALUE_LEN {
        return Err(FriendError::Failed(format!(
            "friend request is {} B, over the inbox's {} B",
            frame.len(),
            request_inbox::MAX_SUBKEY_VALUE_LEN
        )));
    }
    records(host)?
        .write_once(
            &blob.inbox.record_key,
            request_inbox::REQUEST,
            frame,
            Some(&blob.inbox.writer),
        )
        .await
        .and_then(|outcome| outcome.require_stored(request_inbox::REQUEST))
        .map_err(|e| FriendError::Failed(format!("request inbox not written: {e}")))
}

/// What landed in one of our inboxes: opened and applied as any inbound
/// 1:1 frame (the invite's admission is the request's own).
pub async fn received(host: &dyn FriendHost, value: &[u8]) {
    if let crate::operations::direct::Routed::Unported(opened) =
        crate::operations::direct::receive(host, value).await
    {
        tracing::debug!(kind = ?opened.type_id, "unported kind in a request inbox — dropped");
    }
}

/// Drop `invite_id`'s inbox: it left `pending`.
pub(crate) async fn drop_for(host: &dyn FriendHost, invite_id: &str) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let id = invite_id.to_string();
    let key = state::db_call!(host, "read invite inbox", move |conn| {
        rekindle_db::repo::outgoing_invites::inbox_of(conn, &owner, &id)
    })
    .await;
    if let Ok(Some(key)) = key {
        host.drop_request_inbox(&key).await;
    }
}
