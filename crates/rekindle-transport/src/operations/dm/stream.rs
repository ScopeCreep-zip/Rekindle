//! The DM chain (plan E2.5, MSR + D23): each direction of a conversation is
//! the sender's append-only chain on their dm participant record, and the
//! `rekindle-sync` stream engine reads it. Our frames are appended to our
//! chain before their `app_message` leaves, and that message is only a hint
//! that triggers the peer's diff: the chain is authoritative.
//!
//! - Sender: [`append`] puts each wired frame on our chain; the first one
//!   creates our participant record (`FriendHost::participant_writer`) and
//!   commits its key to the peer as a ratcheted `StreamRecord`.
//! - Receiver: [`on_stream_record`] keeps an accepted friend's record key;
//!   [`on_hint`], [`on_watch_change`] and [`catch_up_all`] diff their chain
//!   from our cursor through [`DmSink`], which feeds each entry to the one
//!   1:1 receive path (a copy we already opened is the ratchet's
//!   duplicate) and then commits the cursor; their `CaughtUp` releases our
//!   retry requests to them, and a retired stretch is a `DmHistoryGap`.

use rekindle_codec::capnp_codec::direct::{DirectBody, StreamRecord};
use rekindle_records::chain::{self, ChainEntry};
use rekindle_records::port::RecordPort;
use rekindle_sync::stream::{SinkError, StreamSink, DM_STREAM};
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

use crate::operations::direct;
use crate::operations::friend::inbox::records;
use crate::operations::friend::{state, FriendError, FriendHost};

/// Our participant record for one conversation, writable.
#[derive(Clone, PartialEq, Eq)]
pub struct ParticipantWriter {
    pub record_key: String,
    /// The record's owner keypair.
    pub writer: String,
    /// Created by this call: its key still has to reach the peer.
    pub created: bool,
}

/// The owner keypair is never printed.
impl std::fmt::Debug for ParticipantWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticipantWriter")
            .field("record_key", &self.record_key)
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

/// Append `frames` (the wired frames of our `dm` rows to `to`, each with
/// its `stream_seq`, in order) to our chain to them, as few pages as hold
/// them ([`chain::page_runs`]: a batch costs two sets per page, not per
/// frame, plan E2.B F2), note each entry, and retire what may go
/// ([`retire`]). A record created for it has its key committed to `to`
/// first. How many frames, from the first, are on the chain, and the error
/// that stopped the rest.
///
/// A frame not counted is not on the chain and must not be counted as
/// sent: the record could not be made or its page did not land.
pub async fn append(
    host: &dyn FriendHost,
    to: &str,
    frames: &[(Vec<u8>, i64)],
) -> (usize, Result<(), FriendError>) {
    let mut landed = 0;
    let result = append_runs(host, to, frames, &mut landed).await;
    (landed, result)
}

async fn append_runs(
    host: &dyn FriendHost,
    to: &str,
    frames: &[(Vec<u8>, i64)],
    landed: &mut usize,
) -> Result<(), FriendError> {
    let ours = host.participant_writer(to).await?;
    if ours.created {
        send_record(host, to, &ours.record_key).await?;
    }
    let port = records(host)?;
    super::chain_seq::append_assigned(host, port.as_ref(), &ours, to, frames, landed).await?;
    retire(host, to, &ours, port.as_ref()).await;
    Ok(())
}

/// Append `frames` (wired entries, each with its `stream_seq`, in order)
/// to the chain `ours` as few pages as hold them ([`chain::page_runs`]),
/// noting each entry's chain seq under `stream` (its `chain_entries` name:
/// the peer of a dm chain, `group_dm:<id>` of a group chain). `landed`
/// counts the frames, from the first, that are on the chain.
///
/// # Errors
/// A page did not land or an entry was not noted: the frames from
/// `landed` on are not on the chain.
pub(crate) async fn append_noted(
    host: &dyn FriendHost,
    port: &dyn RecordPort,
    ours: &ParticipantWriter,
    stream: &str,
    frames: &[(Vec<u8>, i64)],
    landed: &mut usize,
) -> Result<(), FriendError> {
    let datagrams: Vec<Vec<u8>> = frames.iter().map(|(d, _)| d.clone()).collect();
    let runs = chain::page_runs(&datagrams)
        .map_err(|e| FriendError::Failed(format!("chain append: {e}")))?;
    for run in runs {
        let page = chain::append(port, &ours.record_key, &ours.writer, run)
            .await
            .map_err(|e| FriendError::NotReady(format!("chain append: {e}")))?;
        let noted: Vec<(u64, i64)> = (page.first_seq..)
            .zip(&frames[*landed..*landed + run.len()])
            .map(|(chain_seq, (_, stream_seq))| (chain_seq, *stream_seq))
            .collect();
        *landed += run.len();
        let (owner, stream, now) = (
            state::owner(host)?,
            stream.to_string(),
            host.clock().now_ms_i64(),
        );
        state::db_call!(host, "note chain entries", move |conn| {
            for (chain_seq, stream_seq) in noted {
                rekindle_db::repo::chain_entries::record(
                    conn, &owner, &stream, chain_seq, stream_seq, now,
                )?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Retire our chain to `to` through the newest entry the peer has
/// acknowledged, with every entry before it, once it is older than
/// `DELIVERY_HORIZON` (D24, ruling Q2b): readers stop at the mark, and a
/// reader still behind it is shown a `HistoryGap`. A failure waits for the
/// next append.
async fn retire(host: &dyn FriendHost, to: &str, ours: &ParticipantWriter, port: &dyn RecordPort) {
    let horizon = i64::try_from(rekindle_db::repo::chain_entries::CHAIN_RETENTION.as_millis())
        .unwrap_or(i64::MAX);
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let cutoff = host.clock().now_ms_i64().saturating_sub(horizon);
    let (o, peer) = (owner.clone(), to.to_string());
    let through = state::db_call!(host, "read the retire point", move |conn| {
        rekindle_db::repo::chain_entries::retirable(conn, &o, &peer, cutoff)
    })
    .await;
    let seq = match through {
        Ok(Some(seq)) => seq,
        Ok(None) => return,
        Err(e) => return tracing::debug!(to = %to, error = %e, "retire point not read"),
    };
    if let Err(e) = chain::retire_through(port, &ours.record_key, &ours.writer, seq).await {
        return tracing::debug!(to = %to, error = %e, "chain not retired");
    }
    let peer = to.to_string();
    let forgot = state::db_call!(host, "forget retired chain entries", move |conn| {
        rekindle_db::repo::chain_entries::forget_through(conn, &owner, &peer, seq)
    })
    .await;
    if let Err(e) = forgot {
        tracing::debug!(to = %to, error = %e, "retired chain entries not forgotten");
    }
    tracing::info!(to = %to, through = seq, "dm chain retired");
}

/// The friend maintenance's retire tick (plan E2.5): retire each accepted
/// friend's chain that may go ([`retire`]), so a quiet conversation's
/// acknowledged entries past `DELIVERY_HORIZON` go without waiting for our
/// next append. Only a conversation with noted entries and a record of
/// ours held (`FriendHost::held_participant_writer`) is retired; the tick
/// never creates or sends a record.
pub async fn retire_quiet(host: &dyn FriendHost) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let peers = match state::db_call!(host, "list chain peers", move |conn| {
        rekindle_db::repo::chain_entries::peers(conn, &owner)
    })
    .await
    {
        Ok(peers) => peers,
        Err(e) => return tracing::debug!(error = %e, "chain peers not read"),
    };
    let Ok(port) = records(host) else {
        return;
    };
    for peer in peers.iter().filter(|p| state::is_accepted(host, p)) {
        match host.held_participant_writer(peer).await {
            Ok(Some(ours)) => {
                // A sealed entry whose row left (a deadline, a wedge) still
                // lands (finding 71 refined), with no later message to carry it.
                if let Err(e) =
                    super::chain_seq::append_pending(host, port.as_ref(), &ours, peer).await
                {
                    tracing::debug!(to = %peer, error = %e, "pending chain entries not appended");
                }
                retire(host, peer, &ours, port.as_ref()).await;
            }
            // No record of ours held: nothing to retire (the tick never
            // creates or sends one).
            Ok(None) => {}
            Err(e) => tracing::debug!(to = %peer, error = %e, "participant record unreadable"),
        }
    }
}

/// The client id of the `dm` row that carries `record_key` in a
/// `StreamRecord`: both sides derive it from the key, so the receiver's
/// `received` row and the sender's outbox row name the same entry.
#[must_use]
pub fn stream_record_client_id(record_key: &str) -> String {
    format!(
        "StreamRecord:{}",
        blake3::hash(record_key.as_bytes()).to_hex()
    )
}

/// Commit our participant record's key to `to` as a row of our `dm` stream
/// (sent once a session exists, and again after the record is replaced):
/// the outbox resends it until the peer's `DmAck` names it.
pub(super) async fn send_record(
    host: &dyn FriendHost,
    to: &str,
    record_key: &str,
) -> Result<(), FriendError> {
    let body = DirectBody::StreamRecord(StreamRecord {
        record_key: record_key.to_string(),
        group_id: None,
    });
    let payload = super::outbox::payload(&body)?;
    host.outbox()?
        .enqueue_control(to, stream_record_client_id(record_key), payload)
        .await
        .map(|_| ())
}

/// A friend's `StreamRecord`: kept for an accepted friend only; a new key
/// restarts our cursor on them and diffs it. Whether it is done.
pub(crate) async fn on_stream_record(
    host: &dyn FriendHost,
    sender: &str,
    body: &StreamRecord,
) -> bool {
    if !state::is_accepted(host, sender) {
        tracing::debug!(from = %sender, "ignoring StreamRecord from non-friend");
        return true;
    }
    if body.group_id.is_some() {
        // A member's group chain (plan E2.6) is `rekindle-dm`'s to follow,
        // from slice G2; it is never this friend's DM record.
        tracing::debug!(from = %sender, "group StreamRecord not applied before E2.6 G2");
        return true;
    }
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let (peer, key) = (sender.to_string(), body.record_key.clone());
    let stored = state::db_call!(host, "store peer participant record", move |conn| {
        let changed =
            rekindle_db::repo::friends::set_peer_participant_record_key(conn, &owner, &peer, &key)?;
        if changed {
            rekindle_db::repo::stream_cursors::reset(conn, &owner, DM_STREAM, &peer)?;
        }
        Ok(changed)
    })
    .await;
    match stored {
        Ok(changed) => {
            if changed {
                let (owned, peer, record) =
                    (host.owned(), sender.to_string(), body.record_key.clone());
                host.scope()
                    .spawn_or_drop("watch participant record", async move {
                        owned.watch_participant(&peer, &record).await;
                    });
                spawn_diff(host, sender);
            }
            true
        }
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "peer participant record not stored");
            false
        }
    }
}

/// An `app_message` from `peer` arrived: diff their chain in the
/// background.
pub fn on_hint(host: &dyn FriendHost, peer: &str) {
    spawn_diff(host, peer);
}

/// `peer`'s participant record `record` changed (its watch fired): diff it
/// when it is the record we hold for them.
pub async fn on_watch_change(host: &dyn FriendHost, peer: &str, record: &str) {
    match peer_record(host, Some(peer)).await.as_deref() {
        Ok([(_, held)]) if held == record => diff(host, peer).await,
        _ => tracing::debug!(peer = %peer, "change on a record we do not read"),
    }
}

/// Diff every accepted friend's chain (startup, a route change, a watch
/// death).
pub async fn catch_up_all(host: &dyn FriendHost) {
    let peers = match peer_record(host, None).await {
        Ok(peers) => peers,
        Err(e) => return tracing::debug!(error = %e, "no streams to catch up"),
    };
    for (peer, _) in peers {
        diff(host, &peer).await;
    }
}

fn spawn_diff(host: &dyn FriendHost, peer: &str) {
    let (owned, peer) = (host.owned(), peer.to_string());
    host.scope().spawn_or_drop("dm stream diff", async move {
        diff(owned.as_ref(), &peer).await;
    });
}

async fn peer_record(
    host: &dyn FriendHost,
    peer: Option<&str>,
) -> Result<Vec<(String, String)>, FriendError> {
    let owner = state::owner(host)?;
    let peer = peer.map(ToString::to_string);
    state::db_call!(host, "read peer participant records", move |conn| {
        rekindle_db::repo::friends::peer_participant_record_key(conn, &owner, peer.as_deref())
    })
    .await
}

/// Diff `peer`'s chain to us from our cursor; a failure waits for the
/// next trigger, which resumes from the committed cursor.
async fn diff(host: &dyn FriendHost, peer: &str) {
    let record = match peer_record(host, Some(peer)).await {
        Ok(mut found) if found.len() == 1 => found.remove(0).1,
        Ok(_) => return tracing::trace!(peer = %peer, "no participant record of theirs yet"),
        Err(e) => return tracing::debug!(peer = %peer, error = %e, "stream diff skipped"),
    };
    let Ok(port) = records(host) else {
        return tracing::debug!(peer = %peer, "stream diff waits for the records pool");
    };
    let sink = DmSink { host };
    match host
        .stores()
        .streams
        .diff(port.as_ref(), &sink, DM_STREAM, peer, &record)
        .await
    {
        Ok(d) => {
            tracing::debug!(peer = %peer, applied = d.applied, coalesced = d.coalesced, "dm stream diffed");
        }
        Err(e) => {
            tracing::debug!(peer = %peer, error = %e, "dm stream diff failed");
        }
    }
}

/// The DM stream's sink: the 1:1 receive path and `stream_cursors`.
struct DmSink<'a> {
    host: &'a dyn FriendHost,
}

impl DmSink<'_> {
    async fn advance(&self, writer: &str, seq: u64) -> Result<(), SinkError> {
        let owner = state::owner(self.host).map_err(|e| SinkError(e.to_string()))?;
        let (writer, now) = (writer.to_string(), self.host.clock().now_ms_i64());
        state::db_call!(self.host, "commit stream cursor", move |conn| {
            rekindle_db::repo::stream_cursors::advance(conn, &owner, DM_STREAM, &writer, seq, now)
        })
        .await
        .map(|_| ())
        .map_err(|e| SinkError(e.to_string()))
    }
}

#[async_trait::async_trait]
impl StreamSink for DmSink<'_> {
    async fn cursor(&self, stream: &str, writer: &str) -> Result<u64, SinkError> {
        let owner = state::owner(self.host).map_err(|e| SinkError(e.to_string()))?;
        let (stream, writer) = (stream.to_string(), writer.to_string());
        state::db_call!(self.host, "read stream cursor", move |conn| {
            rekindle_db::repo::stream_cursors::get(conn, &owner, &stream, &writer)
        })
        .await
        .map_err(|e| SinkError(e.to_string()))
    }

    /// The entry goes through the one 1:1 receive path: the ratchet and
    /// `unprocessed` row commit before the apply (D24), a copy already
    /// opened is the ratchet's duplicate, and a resend already applied is
    /// the outbox's `received`; so the cursor, committed after, never
    /// loses or doubles an entry.
    async fn apply(
        &self,
        _stream: &str,
        writer: &str,
        entry: &ChainEntry,
    ) -> Result<(), SinkError> {
        if let direct::Routed::Unported(opened) =
            direct::receive_entry(self.host, writer, &entry.ciphertext).await
        {
            tracing::debug!(from = %opened.sender_hex, kind = ?opened.type_id,
                "unported kind on the dm chain — dropped");
        }
        self.advance(writer, entry.seq).await
    }

    async fn history_gap(
        &self,
        _stream: &str,
        writer: &str,
        through: u64,
    ) -> Result<(), SinkError> {
        // D23: the gap is a stored row of the conversation, so it survives
        // a reload; a failed insert keeps the cursor, and the gap is seen
        // again on the next diff.
        let owner = state::owner(self.host).map_err(|e| SinkError(e.to_string()))?;
        let (peer, now) = (writer.to_string(), self.host.clock().now_ms_i64());
        state::db_call!(self.host, "store dm history gap", move |conn| {
            rekindle_db::repo::history_gap::insert(
                conn,
                &owner,
                (&peer, rekindle_types::message::ConversationType::Dm),
                &peer,
                through,
                now,
            )
        })
        .await
        .map_err(|e| SinkError(e.to_string()))?;
        self.host.publish(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::DmHistoryGap {
                peer_key: writer.to_string(),
                through_seq: through,
            },
        ));
        self.advance(writer, through).await
    }

    async fn caught_up(&self, _stream: &str, writer: &str) {
        direct::retry_requests::on_caught_up(self.host, writer).await;
    }
}
