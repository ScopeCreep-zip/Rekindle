//! A dm row's chain seq (live A/B finding 71 refined): when the pump hands
//! a content row to its stream's ordered appender (one append in flight
//! per stream, E2.5c), it is assigned the next chain seq, recorded in
//! `chain_entries`, sealed into its hint as `DmContent.chainSeq`, and
//! appended at exactly that seq ([`chain::append_exact`]). A re-hint, or a
//! pass after a crash between the assignment and the append, reuses the
//! recorded seq. A `StreamRecord` row is not on the chain and has none.

use rekindle_db::repo::chain_entries;
use rekindle_records::chain::{self, ChainError};
use rekindle_records::port::RecordPort;
use rekindle_types::dht_layout::dm_participant;

use super::stream::{send_record, ParticipantWriter};
use crate::operations::friend::{state, FriendError, FriendHost};

/// The chain seq recorded for our `dm` row `stream_seq` to `to`, if any.
///
/// # Errors
/// The storage failed.
pub(crate) async fn assigned(
    host: &dyn FriendHost,
    to: &str,
    stream_seq: i64,
) -> Result<Option<u64>, FriendError> {
    let (owner, peer) = (state::owner(host)?, to.to_string());
    state::db_call!(host, "read a chain seq", move |conn| {
        chain_entries::assigned(conn, &owner, &peer, stream_seq)
    })
    .await
}

/// The chain seq our `dm` row `stream_seq` to `to` goes to: the recorded
/// one, or the next after the conversation's persisted chain head (no DHT
/// read precedes the hint). Our participant record is made first when we
/// hold none (its key committed to `to`), since a new record is a new
/// chain and resets the head.
///
/// # Errors
/// The record could not be made, or the storage failed.
pub(crate) async fn assign(
    host: &dyn FriendHost,
    to: &str,
    stream_seq: i64,
) -> Result<u64, FriendError> {
    if let Some(seq) = assigned(host, to, stream_seq).await? {
        return Ok(seq);
    }
    if host.held_participant_writer(to).await?.is_none() {
        let ours = host.participant_writer(to).await?;
        if ours.created {
            send_record(host, to, &ours.record_key).await?;
        }
    }
    let (owner, peer) = (state::owner(host)?, to.to_string());
    state::db_call!(host, "assign a chain seq", move |conn| {
        chain_entries::assign(conn, &owner, &peer, stream_seq)
    })
    .await?
    .ok_or_else(|| FriendError::NotReady(format!("no friend row numbers our chain to {to}")))
}

/// Entry `chain_seq` is sealed into `frame`, the hint about to leave: it is
/// kept until it lands, whatever happens to its row.
///
/// # Errors
/// The storage failed.
pub(crate) async fn seal(
    host: &dyn FriendHost,
    to: &str,
    chain_seq: u64,
    frame: Vec<u8>,
) -> Result<(), FriendError> {
    let (owner, peer) = (state::owner(host)?, to.to_string());
    state::db_call!(host, "seal a chain entry", move |conn| {
        chain_entries::set_frame(conn, &owner, &peer, chain_seq, &frame)
    })
    .await
}

/// Entry `chain_seq` was assigned and its framing failed: nothing carries
/// it, so the head steps back (a sealed entry is never released).
pub(crate) async fn unseal(host: &dyn FriendHost, to: &str, chain_seq: u64) {
    let Ok(owner) = state::owner(host) else {
        return;
    };
    let peer = to.to_string();
    let released = state::db_call!(host, "release an unsealed chain seq", move |conn| {
        chain_entries::unassign(conn, &owner, &peer, chain_seq)
    })
    .await;
    if let Err(e) = released {
        tracing::debug!(to = %to, chain_seq, error = %e, "unsealed chain seq kept");
    }
}

/// Append `frames` (our `dm` rows to `to` handed to the appender, each with
/// its `stream_seq`, in order) to `ours`: every sealed entry still to land
/// goes on the chain first ([`append_pending`]), so the seqs behind a row
/// that failed after its hint left still land. `landed` counts the frames,
/// from the first, whose entries are on the chain.
///
/// # Errors
/// A page did not land, or the storage failed: the frames from `landed`
/// on are not on the chain.
pub(crate) async fn append_assigned(
    host: &dyn FriendHost,
    port: &dyn RecordPort,
    ours: &ParticipantWriter,
    to: &str,
    frames: &[(Vec<u8>, i64)],
    landed: &mut usize,
) -> Result<(), FriendError> {
    let appended = append_pending(host, port, ours, to).await;
    for (_, stream_seq) in frames {
        let (owner, peer, s) = (state::owner(host)?, to.to_string(), *stream_seq);
        let on = state::db_call!(host, "read a landing", move |conn| {
            chain_entries::has_landed(conn, &owner, &peer, s)
        })
        .await?;
        if !on {
            break;
        }
        *landed += 1;
    }
    appended?;
    if *landed < frames.len() {
        return Err(FriendError::NotReady(
            "chain append: an entry has no sealed frame".into(),
        ));
    }
    Ok(())
}

/// Append every sealed entry of our chain to `to` still to land, in seq
/// order, as few pages as hold them, each run at exactly its first seq
/// ([`chain::append_exact`]), and note each landed. A run the chain already
/// holds (a crash after the page landed, before it was noted) is noted and
/// not written again. The friend maintenance calls it too, so an entry
/// whose row left lands without waiting for a later message.
///
/// # Errors
/// A page did not land, the chain's next entry is not the pending one, or
/// the storage failed.
pub(crate) async fn append_pending(
    host: &dyn FriendHost,
    port: &dyn RecordPort,
    ours: &ParticipantWriter,
    to: &str,
) -> Result<(), FriendError> {
    let (owner, peer) = (state::owner(host)?, to.to_string());
    let pending = state::db_call!(host, "read pending chain entries", move |conn| {
        chain_entries::pending(conn, &owner, &peer)
    })
    .await?;
    let datagrams: Vec<Vec<u8>> = pending.iter().map(|(_, _, f)| f.clone()).collect();
    let runs = chain::page_runs(&datagrams)
        .map_err(|e| FriendError::Failed(format!("chain append: {e}")))?;
    let mut at = 0;
    for run in runs {
        let first = pending[at].0;
        let last = first + (run.len() as u64 - 1);
        if pending[at + run.len() - 1].0 != last {
            return Err(FriendError::Failed(format!(
                "chain append: pending seqs from {first} are not contiguous"
            )));
        }
        let written = chain::append_exact(
            port,
            &ours.record_key,
            dm_participant::HEAD,
            &ours.writer,
            first,
            run,
        )
        .await;
        match written {
            Ok(_) => {}
            Err(ChainError::SeqMoved { next, .. }) if next > last => {
                tracing::debug!(to = %to, first, last, "dm run already on the chain");
            }
            Err(e) => return Err(FriendError::NotReady(format!("chain append: {e}"))),
        }
        at += run.len();
        let (owner, peer, now) = (
            state::owner(host)?,
            to.to_string(),
            host.clock().now_ms_i64(),
        );
        state::db_call!(host, "note chain entries landed", move |conn| {
            chain_entries::landed(conn, &owner, &peer, (first, last), now)
        })
        .await?;
    }
    Ok(())
}
