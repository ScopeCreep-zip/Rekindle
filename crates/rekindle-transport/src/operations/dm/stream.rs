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

/// Append `datagram` (a wired ratchet frame to `to`) to our chain to them.
/// A record created for it has its key committed to `to` first.
///
/// # Errors
/// The record could not be made or the page did not land: the frame is
/// not on the chain and must not be counted as sent.
pub async fn append(host: &dyn FriendHost, to: &str, datagram: &[u8]) -> Result<u64, FriendError> {
    let ours = host.participant_writer(to).await?;
    if ours.created {
        send_record(host, to, &ours.record_key).await?;
    }
    let port = records(host)?;
    let page = chain::append(
        port.as_ref(),
        &ours.record_key,
        &ours.writer,
        &[datagram.to_vec()],
    )
    .await
    .map_err(|e| FriendError::NotReady(format!("chain append: {e}")))?;
    Ok(page.last_seq)
}

/// Commit our participant record's key to `to`'s handshake stream (sent
/// once a session exists, and again after the record is replaced).
async fn send_record(host: &dyn FriendHost, to: &str, record_key: &str) -> Result<(), FriendError> {
    let body = DirectBody::StreamRecord(StreamRecord {
        record_key: record_key.to_string(),
    });
    super::handshake::enqueue(host, to, &body).await.map(|_| ())
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
