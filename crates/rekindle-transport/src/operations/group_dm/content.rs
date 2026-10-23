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
use rekindle_types::subscription_events::TypingContext;

use super::{friend_error, parse_group_id, send_pairwise};
use crate::operations::dm::outbox::{self as dm_outbox, Attempt, OutgoingDm};
use crate::operations::dm::stream as dm_stream;
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

/// One attempt of `rows`, the due run of our stream to `group_id` in
/// `stream_seq` order: each sealed under our current key, the run on our
/// chain at once in as few pages as hold it (plan E2.B F2, shared with the
/// dm stream: `dm::stream::append_noted`), then each hinted to every
/// member. Each attempt carries `sent_at`, when the message was written
/// (its echo's time; now for a row with none), so members order it by that
/// (E2.B F3). The outcomes of a prefix of `rows`, in order: the rows after
/// the first that stays behind (`rekindle_outbox::holds_back`) are not
/// attempted.
pub async fn attempt(
    host: &dyn FriendHost,
    group_id: &str,
    rows: &[dm_outbox::RowAttempt<'_>],
) -> Vec<Attempt> {
    let id = match parse_group_id(group_id) {
        Ok(id) => id,
        Err(e) => {
            return rows
                .iter()
                .take(1)
                .map(|_| Attempt::Wedged(e.to_string()))
                .collect()
        }
    };
    let mut outcomes: Vec<Option<Attempt>> = Vec::with_capacity(rows.len());
    let (mut slots, mut frames, mut sealed) = (Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        // On our chain already (live A/B finding 42): not sealed, appended
        // or hinted again.
        if dm_outbox::landed(row) {
            outcomes.push(Some(Attempt::Sent));
            continue;
        }
        match seal_row(host, &id, row).await {
            Ok(content) => {
                slots.push(outcomes.len());
                outcomes.push(None);
                frames.push((
                    DirectBody::GroupContent(content.clone()).encode(),
                    row.stream_seq,
                ));
                sealed.push(content);
            }
            Err(outcome) => {
                if dm_outbox::push_stops(&mut outcomes, outcome) {
                    break;
                }
            }
        }
    }
    if !frames.is_empty() {
        let (landed, appended) = append(host, &id, group_id, &frames).await;
        sealed.truncate(landed);
        spawn_hints(host, &id, sealed);
        dm_outbox::settle_chained(&mut outcomes, &slots, landed, appended);
    }
    dm_outbox::finish(outcomes)
}

/// Seal `row` under our current sending key for this attempt.
async fn seal_row(
    host: &dyn FriendHost,
    id: &GroupId,
    row: &dm_outbox::RowAttempt<'_>,
) -> Result<GroupContent, Attempt> {
    let body = dm_outbox::decode_payload(row.payload)
        .map_err(|e| Attempt::Wedged(format!("payload: {e}")))?;
    let sent_at_ms = row
        .sent_at
        .and_then(|t| u64::try_from(t).ok())
        .unwrap_or_else(|| host.clock().now_ms());
    let plain = inner(
        &body,
        sent_at_ms,
        u64::try_from(row.stream_seq).unwrap_or(0),
    );
    let deps = host.group_dm().map_err(dm_outbox::attempt_of)?;
    let content = rekindle_dm::seal_for_send(deps.as_ref(), id, &plain)
        .await
        .map_err(|e| dm_outbox::attempt_of(friend_error(&e)))?;
    dm_outbox::fits_page(&DirectBody::GroupContent(content.clone()).encode())?;
    Ok(content)
}

/// Append `frames` (sealed entries, each with its `stream_seq`) to our
/// group chain, noting each entry, and retire what may go. How many frames,
/// from the first, are on the chain, and the error that stopped the rest.
async fn append(
    host: &dyn FriendHost,
    id: &GroupId,
    group_id: &str,
    frames: &[(Vec<u8>, i64)],
) -> (usize, Result<(), FriendError>) {
    let mut landed = 0;
    let result = append_runs(host, id, group_id, frames, &mut landed).await;
    (landed, result)
}

async fn append_runs(
    host: &dyn FriendHost,
    id: &GroupId,
    group_id: &str,
    frames: &[(Vec<u8>, i64)],
    landed: &mut usize,
) -> Result<(), FriendError> {
    let group = held(host, id).await?;
    let ours = host.group_chain_writer(group_id).await?;
    if ours.created {
        announce(host, &group, &ours.record_key).await?;
    }
    let port = records(host)?;
    let stream = stream_name(group_id);
    dm_stream::append_noted(host, port.as_ref(), &ours, &stream, frames, landed).await?;
    super::retire::retire(host, group_id, &ours, port.as_ref()).await;
    Ok(())
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

/// Send each of `contents` (in order) to every other member once, in one
/// background task: only a hint that makes them diff our chain (or, for
/// typing, the whole of it).
fn spawn_hints(host: &dyn FriendHost, id: &GroupId, contents: Vec<GroupContent>) {
    if contents.is_empty() {
        return;
    }
    let (owned, id) = (host.owned(), *id);
    host.scope().spawn_or_drop("group hints", async move {
        let Ok(group) = held(owned.as_ref(), &id).await else {
            return;
        };
        let Ok(deps) = owned.group_dm() else {
            return;
        };
        let others = group.roster.others(&deps.own_identity());
        for content in contents {
            let body = DirectBody::GroupContent(content);
            for to in &others {
                if let Err(e) = send_to_peer(owned.as_ref(), to, &body).await {
                    tracing::debug!(to = %to, error = %e, "group hint not sent");
                }
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
    spawn_hints(host, &id, vec![content]);
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
