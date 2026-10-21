//! Our group content (plan E2.6 G2c). A message to a group is one row of
//! the `group_dm:<group_id>` outbox stream committed with its local echo
//! (a `group_dm` `messages` row under the group's id). Each attempt seals
//! it under our current sending key, appends the `GroupContent` to our
//! group chain (the chain is authoritative, D23) and only then sends the
//! same entry to every other member as a best-effort hint over their
//! pairwise session (no row per hint). The row settles `sent` once the
//! entry landed: a group has no `DmAck` or `ReadUpTo`, so never
//! `delivered` or `read`. Typing is a hint only, never on the chain.

use std::time::Duration;

use rekindle_codec::capnp_codec::direct::{
    DirectBody, DmContent, DmText, DmTypeId, GroupContent, GroupId, StreamRecord, Typing,
};
use rekindle_dm::{GroupState, Outbound};
use rekindle_events::typing::{Input, Lapse};
use rekindle_records::chain;
use rekindle_types::subscription_events::TypingContext;

use super::{friend_error, parse_group_id, send_pairwise};
use crate::operations::dm::outbox::{self as dm_outbox, Attempt, OutgoingDm};
use crate::operations::dm::{client_id_text, hint_for, mint_client_id, send_to_peer};
use crate::operations::friend::inbox::records;
use crate::operations::friend::{state, FriendError, FriendHost};

/// The outbox stream of a group (`group_dm:<group_id>`), also the
/// `chain_entries` name of our chain in it.
#[must_use]
pub fn stream_name(group_id: &str) -> String {
    format!("group_dm:{group_id}")
}

/// The sealed plaintext of a group entry: the kind's typeId (big-endian)
/// and the [`DmContent`] carrying its body and the author's clock.
pub fn inner(body: &DirectBody, sent_at_ms: u64, stream_seq: u64) -> Vec<u8> {
    let type_id = body.type_id();
    let content = DmContent {
        content_hint: hint_for(type_id),
        sent_at_ms,
        payload: body.encode(),
        stream_seq,
    };
    let mut out = type_id.as_u16().to_be_bytes().to_vec();
    out.extend(content.encode());
    out
}

/// [`inner`] read back: the body and the author's clock.
///
/// # Errors
/// Not an [`inner`] of an allocated kind.
pub fn parse_inner(plain: &[u8]) -> Result<(DirectBody, u64), String> {
    let (id, rest) = plain.split_at_checked(2).ok_or("entry too short")?;
    let raw = u16::from_be_bytes([id[0], id[1]]);
    let type_id = DmTypeId::from_u16(raw).ok_or_else(|| format!("typeId {raw:#06x}"))?;
    let content = DmContent::decode(rest).map_err(|e| e.to_string())?;
    let body = DirectBody::decode(type_id, &content.payload).map_err(|e| e.to_string())?;
    Ok((body, content.sent_at_ms))
}

async fn held(host: &dyn FriendHost, id: &GroupId) -> Result<GroupState, FriendError> {
    let deps = host.group_dm()?;
    deps.group(id)
        .await
        .map_err(|e| friend_error(&e))?
        .ok_or_else(|| FriendError::NotFound(format!("no group {}", hex::encode(id))))
}

/// Commit our message `body` to `group_id` (32 hex) with its local echo in
/// one transaction; its client id once committed (`queued`, then the
/// outbox's `GroupDmEvent::SendStatus`).
///
/// # Errors
/// No such group, locked, or the message could not be committed.
pub async fn send(
    host: &dyn FriendHost,
    group_id: &str,
    body: &str,
) -> Result<String, FriendError> {
    held(host, &parse_group_id(group_id)?).await?;
    let owner = state::owner(host)?;
    let client_id = mint_client_id(host);
    let message_id = client_id_text(&client_id);
    let payload = dm_outbox::payload(&DirectBody::DmText(DmText {
        client_id,
        body: body.to_string(),
        reply_to: None,
        expire_seconds: 0,
    }))?;
    let dm = OutgoingDm {
        client_id: message_id.clone(),
        payload,
        sender: owner,
        body: body.to_string(),
        timestamp: host.clock().now_ms_i64(),
        reply_to: None,
    };
    host.outbox()?.enqueue_group(group_id, dm).await?;
    // Sending the message ends our typing in the group.
    host.stores()
        .typing_out
        .lock()
        .end(&typing_context(group_id));
    Ok(message_id)
}

/// One attempt of row `stream_seq` of our stream to `group_id`: sealed
/// under our current key, on our chain, then hinted to each member.
pub async fn attempt(
    host: &dyn FriendHost,
    group_id: &str,
    stream_seq: i64,
    payload: &[u8],
) -> Attempt {
    let body = match dm_outbox::decode_payload(payload) {
        Ok(body) => body,
        Err(e) => return Attempt::Wedged(format!("payload: {e}")),
    };
    let id = match parse_group_id(group_id) {
        Ok(id) => id,
        Err(e) => return Attempt::Wedged(e.to_string()),
    };
    match append(host, &id, group_id, &body, stream_seq).await {
        Ok(content) => {
            spawn_hints(host, &id, content);
            Attempt::Sent
        }
        Err(FriendError::Locked | FriendError::NotReady(_) | FriendError::Conflict(_)) => {
            Attempt::Deferred
        }
        Err(e) => Attempt::Wedged(e.to_string()),
    }
}

/// Seal `body` and append it to our group chain, noting the entry; the
/// entry.
async fn append(
    host: &dyn FriendHost,
    id: &GroupId,
    group_id: &str,
    body: &DirectBody,
    stream_seq: i64,
) -> Result<GroupContent, FriendError> {
    let group = held(host, id).await?;
    let deps = host.group_dm()?;
    let plain = inner(
        body,
        host.clock().now_ms(),
        u64::try_from(stream_seq).unwrap_or(0),
    );
    let content = rekindle_dm::seal_for_send(deps.as_ref(), id, &plain)
        .await
        .map_err(|e| friend_error(&e))?;
    let ours = host.group_chain_writer(group_id).await?;
    if ours.created {
        announce(host, &group, &ours.record_key).await?;
    }
    let port = records(host)?;
    let entry = DirectBody::GroupContent(content.clone()).encode();
    let page = chain::append(port.as_ref(), &ours.record_key, &ours.writer, &[entry])
        .await
        .map_err(|e| FriendError::NotReady(format!("group chain append: {e}")))?;
    let (owner, stream, now) = (
        state::owner(host)?,
        stream_name(group_id),
        host.clock().now_ms_i64(),
    );
    let chain_seq = page.last_seq;
    state::db_call!(host, "note group chain entry", move |conn| {
        rekindle_db::repo::chain_entries::record(conn, &owner, &stream, chain_seq, stream_seq, now)
    })
    .await?;
    super::retire::retire(host, group_id, &ours, port.as_ref()).await;
    Ok(content)
}

/// Our group chain was made again (its owner keypair was not held): it is
/// ours from now on, and announced to every other member pairwise as a
/// `StreamRecord{group_id}` (the decided 2026-10-21 rule).
async fn announce(
    host: &dyn FriendHost,
    group: &GroupState,
    record_key: &str,
) -> Result<(), FriendError> {
    let deps = host.group_dm()?;
    let me = deps.own_identity();
    deps.put_chain(&group.group_id, &me, record_key)
        .await
        .map_err(|e| friend_error(&e))?;
    let out = Outbound {
        frame: StreamRecord {
            record_key: record_key.to_owned(),
            group_id: Some(group.group_id),
        },
        to: group.roster.others(&me),
    };
    send_pairwise(host, out, DirectBody::StreamRecord).await
}

/// Send `content` to every other member once, in the background: only a
/// hint that makes them diff our chain (or, for typing, the whole of it).
fn spawn_hints(host: &dyn FriendHost, id: &GroupId, content: GroupContent) {
    let (owned, id) = (host.owned(), *id);
    host.scope().spawn_or_drop("group hints", async move {
        let Ok(group) = held(owned.as_ref(), &id).await else {
            return;
        };
        let Ok(deps) = owned.group_dm() else {
            return;
        };
        let body = DirectBody::GroupContent(content);
        for to in group.roster.others(&deps.own_identity()) {
            if let Err(e) = send_to_peer(owned.as_ref(), &to, &body).await {
                tracing::debug!(to = %to, error = %e, "group hint not sent");
            }
        }
    });
}

fn typing_context(group_id: &str) -> TypingContext {
    TypingContext::GroupDm {
        group_id: group_id.to_owned(),
    }
}

/// Our typing state to every member of `group_id`: sealed like content
/// and sent as a hint only (never on the chain, never stored).
async fn send_typing(
    host: &dyn FriendHost,
    group_id: &str,
    typing: bool,
) -> Result<(), FriendError> {
    let id = parse_group_id(group_id)?;
    let deps = host.group_dm()?;
    let plain = inner(
        &DirectBody::Typing(Typing { typing }),
        host.clock().now_ms(),
        0,
    );
    let content = rekindle_dm::seal_for_send(deps.as_ref(), &id, &plain)
        .await
        .map_err(|e| friend_error(&e))?;
    spawn_hints(host, &id, content);
    Ok(())
}

/// The window's draft in `group_id` changed: `typing` is whether it holds
/// text. The session is E2.3's (`typing_out`): a start, a refresh every
/// `TYPING_REFRESH_MS` of input, and the stop on a cleared draft or idle.
///
/// # Errors
/// Not a group we hold, or the start or stop could not be sealed.
pub async fn report_draft(
    host: &dyn FriendHost,
    group_id: &str,
    typing: bool,
) -> Result<(), FriendError> {
    let ctx = typing_context(group_id);
    if !typing {
        let ended = host.stores().typing_out.lock().end(&ctx);
        return if ended {
            send_typing(host, group_id, false).await
        } else {
            Ok(())
        };
    }
    let now = host.clock().now_ms();
    let input = host.stores().typing_out.lock().input(ctx, now);
    match input {
        Input::Started => {
            watch_idle(host, group_id);
            send_typing(host, group_id, true).await
        }
        Input::Refresh => send_typing(host, group_id, true).await,
        Input::Quiet => Ok(()),
    }
}

/// Send the stop once the group's session goes idle.
fn watch_idle(host: &dyn FriendHost, group_id: &str) {
    let (owned, group) = (host.owned(), group_id.to_string());
    host.scope()
        .spawn_or_drop("group typing idle stop", async move {
            let ctx = typing_context(&group);
            loop {
                let now = owned.clock().now_ms();
                let lapse = owned.stores().typing_out.lock().idle(&ctx, now);
                match lapse {
                    Lapse::Pending(deadline) => {
                        tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now)))
                            .await;
                    }
                    Lapse::Ended => {
                        if let Err(e) = send_typing(owned.as_ref(), &group, false).await {
                            tracing::debug!(group = %group, error = %e, "typing stop not sent");
                        }
                        return;
                    }
                    Lapse::Gone => return,
                }
            }
        });
}
