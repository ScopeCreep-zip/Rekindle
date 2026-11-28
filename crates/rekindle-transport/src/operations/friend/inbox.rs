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

/// The session's records pool.
///
/// # Errors
/// The node or its pool is not up.
pub(crate) fn records(
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
    let (record_key, owner, lease, writer) = new_inbox(host).await?;
    open(host, lease).await?;
    host.keep_request_inbox(&record_key, &owner, lease).await?;
    Ok(InboxCapability { record_key, writer })
}

/// Put a new inbox on the network by its owner subkey's one write (an
/// invite's is shown, and a friend code's published, only once its inbox
/// can be written); a write that misses releases `lease`.
///
/// # Errors
/// The write did not reach consensus.
pub(crate) async fn open(
    host: &dyn FriendHost,
    lease: rekindle_records::lease::LeaseId,
) -> Result<(), FriendError> {
    let pool = records(host)?;
    let opened = pool
        .set(
            lease,
            request_inbox::OWNER_SUBKEY,
            request_inbox::OPENED.to_vec(),
            None,
        )
        .await
        .and_then(|outcome| outcome.require_stored(request_inbox::OWNER_SUBKEY));
    if let Err(e) = opened {
        pool.release(lease).await;
        return Err(FriendError::Failed(format!(
            "request inbox not opened: {e}"
        )));
    }
    Ok(())
}

/// Create our request's reply inbox (plan RV2): the same shape as an
/// invite's, a fresh one-time writer the request carries sealed to its
/// addressee, so only they can write it, once. The Host holds and watches
/// it while the request is open (`FriendHost::keep_reply_inbox`) and
/// releases it when the request settles (`FriendHost::sync_reply_inboxes`).
///
/// It is created locally, which needs no network (Veilid's `create_record`
/// stores the new record in the local store only,
/// `storage_manager/create_record.rs`), so a request made offline is
/// committed as before; the owner subkey's write that puts the inbox on
/// the network is the first step of the request's own outbox row
/// ([`reply_slot`]), so the request is never sent before its reply slot
/// exists.
///
/// # Errors
/// The record could not be created or its owner keypair kept.
pub(crate) async fn create_reply(host: &dyn FriendHost) -> Result<InboxCapability, FriendError> {
    let (record_key, owner, lease, writer) = new_inbox(host).await?;
    host.keep_reply_inbox(&record_key, &owner, lease).await?;
    Ok(InboxCapability { record_key, writer })
}

/// Put the reply inbox our open request to `to` carries on the network
/// (plan RV2): its owner subkey's one write, as its owner, the first step
/// of each attempt of the request's row. `None` once it is stored (Veilid
/// stored it at consensus: `Landed`, or `Unchanged` when an earlier attempt
/// did), so the request goes; otherwise what the attempt comes to.
///
/// The pool writes with `allow_offline: false` (`rekindle-record-pool`
/// lib docs, V20): offline, or below consensus, Veilid stores and queues
/// nothing and answers `TryAgain` (`storage_manager/set_value.rs:248-256`,
/// `:731-738` → `handle_offline_set_single_local_value_with_subkey_lock`),
/// reported `Offline`, so this row's own backoff re-attempts it and the
/// outbox stays the one owner of durability (D22): no second queue in
/// Veilid's `offline_subkey_writes` beside the row.
pub(crate) async fn reply_slot(
    host: &dyn FriendHost,
    to: &str,
) -> Option<crate::operations::dm::outbox::Attempt> {
    use crate::operations::dm::outbox::Attempt;
    use rekindle_records::port::SetOutcome;
    let owner = match state::owner(host) {
        Ok(owner) => owner,
        Err(e) => return Some(deferred(&e)),
    };
    let peer = to.to_owned();
    let key = state::db_call!(host, "read reply inbox", move |conn| {
        rekindle_db::repo::friends::reply_inbox_of(conn, &owner, &peer)
    })
    .await;
    let key = match key {
        Ok(Some(key)) => key,
        Ok(None) => return unslotted(host, to, "our request has no reply inbox").await,
        Err(e) => return Some(deferred(&e)),
    };
    let writer = match host.reply_inbox_owner(&key) {
        Ok(Some(writer)) => writer,
        Ok(None) => return unslotted(host, to, "the reply inbox's owner is gone").await,
        Err(e) => return Some(deferred(&e)),
    };
    let pool = match records(host) {
        Ok(pool) => pool,
        Err(e) => return Some(deferred(&e)),
    };
    let subkey = request_inbox::OWNER_SUBKEY;
    match pool
        .write_once(
            &key,
            subkey,
            request_inbox::OPENED.to_vec(),
            Some(writer.as_str()),
        )
        .await
    {
        Ok(SetOutcome::Landed | SetOutcome::Unchanged) => None,
        Ok(SetOutcome::Superseded(_)) => Some(Attempt::Wedged(
            "the reply inbox holds another owner's value".to_owned(),
        )),
        Ok(outcome @ SetOutcome::Offline) => {
            tracing::debug!(
                ?outcome,
                "reply inbox not on the network yet — request deferred"
            );
            Some(Attempt::Deferred)
        }
        Err(e) if dm::handshake::refuses(&e) => {
            Some(Attempt::Wedged(format!("reply inbox write refused: {e}")))
        }
        Err(e) => {
            tracing::debug!(error = %e, "reply inbox write missed — request deferred");
            Some(Attempt::Deferred)
        }
    }
}

/// Our request to `to` found no reply slot to write (`why`). The higher
/// side of a crossing (`accept_crossing_request`) is the one case where it
/// still goes: its row turned `accepted`, which released the inbox, while
/// its request goes on until it lands (finding 56 corrected) and the
/// peer's accept settles it; that accept is the crossing's own first
/// contact, which never writes our reply inbox (a crossing keeps no peer
/// reply), so no slot is needed. Read from the row itself, so a crossing
/// stored a moment before this attempt is seen. Any other request is never
/// sent without its reply slot.
async fn unslotted(
    host: &dyn FriendHost,
    to: &str,
    why: &str,
) -> Option<crate::operations::dm::outbox::Attempt> {
    let owner = match state::owner(host) {
        Ok(owner) => owner,
        Err(e) => return Some(deferred(&e)),
    };
    let peer = to.to_owned();
    let crossed = state::db_call!(host, "read crossed request", move |conn| {
        rekindle_db::repo::friends::is_accepted_row(conn, &owner, &peer)
    })
    .await;
    match crossed {
        Ok(true) => None,
        Ok(false) => {
            tracing::debug!(to = %to, why, "request wedged: no reply slot");
            Some(crate::operations::dm::outbox::Attempt::Wedged(
                why.to_owned(),
            ))
        }
        Err(e) => Some(deferred(&e)),
    }
}

fn deferred(e: &dyn std::fmt::Display) -> crate::operations::dm::outbox::Attempt {
    tracing::debug!(reason = %e, "request deferred: its reply inbox cannot be written yet");
    crate::operations::dm::outbox::Attempt::Deferred
}

/// A new request inbox with a fresh one-time writer, created locally: its
/// key, owner keypair, lease and the writer's keypair text.
async fn new_inbox(host: &dyn FriendHost) -> Result<NewInbox, FriendError> {
    let mut secret = Zeroizing::new([0u8; 32]);
    host.rng().fill(secret.as_mut_slice());
    let signing = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret);
    new_inbox_for(host, &signing).await
}

/// A new inbox, created locally: its key, owner keypair, lease and the
/// keypair text of its one-time writer `signing`.
pub(crate) type NewInbox = (
    String,
    Zeroizing<String>,
    rekindle_records::lease::LeaseId,
    String,
);

/// [`new_inbox`] for a given writer: a friend code's (plan RV2e) is derived
/// from its token, so every slot of one code has the same writer.
///
/// # Errors
/// The record could not be created.
pub(crate) async fn new_inbox_for(
    host: &dyn FriendHost,
    signing: &rekindle_secrets::ed25519_dalek::SigningKey,
) -> Result<NewInbox, FriendError> {
    let writer = rekindle_crypto::keypair_text::keypair_text_from_signing(signing);
    let schema = rekindle_records::schema::request_inbox(&signing.verifying_key().to_bytes());
    let pool = records(host)?;
    let (lease, record_key, owner) = pool
        .create(&schema, None)
        .await
        .map_err(|e| FriendError::Failed(format!("request inbox not created: {e}")))?;
    Ok((record_key, owner, lease, writer))
}

/// Redeem `blob`: our sealed `FriendRequest`, admitted by `invite_id`,
/// committed to our handshake stream to the inviter as an inbox write
/// (plan E2.5): the outbox writes it into the invite's inbox as the
/// one-time writer, again while the network misses it, and fails it
/// visibly when the inbox already holds another requester's request.
///
/// # Errors
/// Our profile or route is not published yet, the frame cannot be sealed
/// or is over the inbox's cap, or the row could not be stored.
pub(crate) async fn redeem(
    host: &dyn FriendHost,
    blob: &InviteBlob,
    invite_id: String,
    message: &str,
) -> Result<(), FriendError> {
    let request = super::request::build(host, message, RequestAuth::Invite(invite_id)).await?;
    super::request::remember_reply(host, &blob.public_key, &request).await?;
    let request = super::request::named_chain(host, &blob.public_key, request).await?;
    let frame = dm::reply_frame(host, &blob.public_key, &request).await?;
    if frame.len() > request_inbox::MAX_SUBKEY_VALUE_LEN {
        return Err(FriendError::Failed(format!(
            "friend request is {} B, over the inbox's {} B",
            frame.len(),
            request_inbox::MAX_SUBKEY_VALUE_LEN
        )));
    }
    let inbox = &blob.inbox;
    dm::handshake::enqueue_inbox(
        host,
        &blob.public_key,
        &inbox.record_key,
        &inbox.writer,
        frame,
    )
    .await
    .map(drop)
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

/// What landed in one of our requests' reply inboxes (plan RV2): the
/// accepter's first contact, opened through the same admission as its call
/// copy (`direct::receive_reply`); a copy arriving both ways is a duplicate
/// there.
pub async fn received_reply(host: &dyn FriendHost, value: &[u8]) {
    if let crate::operations::direct::Routed::Unported(opened) =
        crate::operations::direct::receive_reply(host, value).await
    {
        tracing::debug!(kind = ?opened.type_id, "unported kind in a reply inbox — dropped");
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
