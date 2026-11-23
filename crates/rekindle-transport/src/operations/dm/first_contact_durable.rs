//! A first contact's durable copy (plan RV2, lead decision 66; three-path
//! delivery for the first contact, as Signal and Session keep a message the
//! live path missed): the accepter's PreKeyInit carrying our `FriendAccept`
//! goes by `app_call` **and** is written once into the reply inbox the
//! request named, as its one-time writer. Before the friendship the call is
//! the only other path, and when every pooled safety route of ours loses to
//! the requester's route (Veilid as built) it stalls for minutes; the
//! requester watches its reply inbox, so the write reaches it anyway.
//!
//! Both copies are the same datagram, and the requester opens the durable
//! one through the same admission as the call copy (`frame::open`), so a
//! copy arriving both ways is the replay store's or the ratchet's duplicate.
//!
//! The write's limits: only the holder of the writer key (the addressee of
//! the sealed request) can write (Veilid refuses any other writer,
//! `storage_manager/schema.rs` `check_subkey_value_data`); the slot holds
//! one value, so one request gives one durable copy; the write is signed by
//! the one-time writer key, never our identity; and the record pool reaches
//! the DHT over our safety route.

use super::handshake::{attempt_call, attempt_inbox, INBOX_TAKEN};
use super::outbox::Attempt;
use super::send::Framed;
use crate::operations::friend::{FriendError, FriendHost};
use rekindle_codec::capnp_codec::direct::{DmFrame, DmTypeId};
use rekindle_types::dht_layout::request_inbox;

/// The datagram of `frame` to `to` as a first contact's row: written into
/// their reply inbox as well when their request named one (plan RV2),
/// taken from their row so it is written at most once.
///
/// # Errors
/// The frame is over the cap, or their row could not be read.
pub(super) async fn framed(
    host: &dyn FriendHost,
    to: &str,
    frame: &DmFrame,
    datagram: Vec<u8>,
) -> Result<Framed, FriendError> {
    if frame.type_id != DmTypeId::PreKeyInit || datagram.len() > request_inbox::MAX_SUBKEY_VALUE_LEN
    {
        return Ok(Framed::Wire { datagram });
    }
    let first = match &frame.body {
        rekindle_codec::capnp_codec::direct::FrameBody::PreKeyInit(init) => init.first_type_id,
        _ => return Ok(Framed::Wire { datagram }),
    };
    if first != DmTypeId::FriendAccept {
        return Ok(Framed::Wire { datagram });
    }
    let owner = crate::operations::friend::state::owner(host)?;
    let pk = to.to_owned();
    let reply = crate::operations::friend::state::db_call!(host, "take reply inbox", move |conn| {
        rekindle_db::repo::friends::take_peer_reply(conn, &owner, &pk)
    })
    .await?;
    Ok(match reply {
        Some((record_key, writer)) => Framed::FirstContactDurable {
            datagram,
            record_key,
            writer,
        },
        None => Framed::Wire { datagram },
    })
}

/// One attempt of a durable first contact: the call and the write go
/// together. The call's receipt settles the row at once; otherwise the row
/// waits for the call to end (its own deadline bounds it) and settles on the
/// write stored, as an invite redemption settles `sent`. A write landing
/// first never cuts the call short: its copy over our General route is what
/// tells the requester its route still carries calls (evidence source 6,
/// `InboundLiveness::reply_landed`), and the inbound it would have heard.
/// Neither landing, the call's answer decides (a refusal ends the row, a
/// miss is attempted again, both copies again).
pub(super) async fn attempt(
    host: &dyn FriendHost,
    to: &str,
    datagram: &[u8],
    record_key: &str,
    writer: &str,
) -> Attempt {
    attempt_as(
        host,
        to,
        datagram,
        record_key,
        writer,
        DmTypeId::FriendAccept,
    )
    .await
}

/// One attempt of our request to `to` sent with their friend code (plan
/// RV2e): the call and the write into the slot their profile publishes go
/// together, as an accept's do ([`attempt`]). `None`: not a code's request,
/// or no slot published, so the row's call goes as before.
pub(super) async fn code_request(
    host: &dyn FriendHost,
    to: &str,
    framed: &super::send::Framed,
    datagram: &[u8],
) -> Option<Attempt> {
    let (key, writer) = crate::operations::friend::code_inbox::slot(host, to, framed).await?;
    let kind = DmTypeId::FriendRequest;
    Some(attempt_as(host, to, datagram, &key, &writer, kind).await)
}

/// [`attempt`] for a first contact of `kind`.
async fn attempt_as(
    host: &dyn FriendHost,
    to: &str,
    datagram: &[u8],
    record_key: &str,
    writer: &str,
    kind: DmTypeId,
) -> Attempt {
    let call = attempt_call(host, to, datagram.to_vec(), kind);
    let write = attempt_inbox(host, record_key, writer, datagram);
    tokio::pin!(call, write);
    let (mut called, mut written) = (None, None);
    loop {
        tokio::select! {
            outcome = &mut call, if called.is_none() => {
                if matches!(outcome, Attempt::Receipted) {
                    return outcome;
                }
                called = Some(outcome);
            }
            outcome = &mut write, if written.is_none() => written = Some(outcome),
        }
        // Each future is polled until it completes, never after.
        if let (Some(called), Some(written)) = (&called, &written) {
            return match written {
                Attempt::Sent => {
                    tracing::info!("first contact landed in the requester's reply inbox");
                    Attempt::Sent
                }
                Attempt::Wedged(why) => {
                    if why != INBOX_TAKEN {
                        tracing::debug!(to = %to, %why, "reply inbox write refused");
                    }
                    called.clone()
                }
                _ => called.clone(),
            };
        }
    }
}
