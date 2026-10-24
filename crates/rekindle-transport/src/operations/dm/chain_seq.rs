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
use crate::operations::friend::inbox::records;
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
/// one, or the next after our chain's head and every pending seq. Our
/// participant record is made first (its key committed to `to`), since a
/// new record is a new chain and forgets the old one's entries. `head`
/// caches the head across one pass's rows.
///
/// # Errors
/// The record could not be made, its head not read, or the storage failed.
pub(crate) async fn assign(
    host: &dyn FriendHost,
    to: &str,
    stream_seq: i64,
    head: &mut Option<u64>,
) -> Result<u64, FriendError> {
    if let Some(seq) = assigned(host, to, stream_seq).await? {
        return Ok(seq);
    }
    let last = if let Some(last) = *head {
        last
    } else {
        let ours = host.participant_writer(to).await?;
        if ours.created {
            send_record(host, to, &ours.record_key).await?;
        }
        let port = records(host)?;
        let read = chain::head(port.as_ref(), &ours.record_key).await;
        let last = read
            .map_err(|e| FriendError::NotReady(format!("chain head: {e}")))?
            .map_or(0, |p| p.last_seq);
        *head = Some(last);
        last
    };
    let (owner, peer) = (state::owner(host)?, to.to_string());
    state::db_call!(host, "assign a chain seq", move |conn| {
        chain_entries::assign(conn, &owner, &peer, stream_seq, last)
    })
    .await
}

/// Append `frames` (our `dm` rows' wired frames to `to`, each with its
/// `stream_seq`, in order) to `ours` as few pages as hold them, each run
/// at exactly the chain seq its first frame's hint carries, and note each
/// landed. A run the chain already holds (a crash after the page landed,
/// before it was noted) is noted and not written again; a hole below the
/// run (an assignment released when its row left unlanded) releases every
/// unlanded seq, and the next pass assigns and hints them again. `landed`
/// counts the frames, from the first, that are on the chain.
///
/// # Errors
/// A frame has no seq or its seqs are not contiguous, a page did not
/// land, or the storage failed: the frames from `landed` on are not on
/// the chain.
pub(crate) async fn append_assigned(
    host: &dyn FriendHost,
    port: &dyn RecordPort,
    ours: &ParticipantWriter,
    to: &str,
    frames: &[(Vec<u8>, i64)],
    landed: &mut usize,
) -> Result<(), FriendError> {
    let mut seqs = Vec::with_capacity(frames.len());
    for (_, stream_seq) in frames {
        seqs.push(assigned(host, to, *stream_seq).await?);
    }
    let datagrams: Vec<Vec<u8>> = frames.iter().map(|(d, _)| d.clone()).collect();
    let runs = chain::page_runs(&datagrams)
        .map_err(|e| FriendError::Failed(format!("chain append: {e}")))?;
    for run in runs {
        let first = run_seq(&seqs[*landed..*landed + run.len()])?;
        let last = first + (run.len() as u64 - 1);
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
            Err(ChainError::SeqMoved { expected, next }) => {
                if next < expected {
                    release(host, to).await?;
                }
                return Err(FriendError::NotReady(format!(
                    "chain append: next entry {next}, assigned {expected}"
                )));
            }
            Err(e) => return Err(FriendError::NotReady(format!("chain append: {e}"))),
        }
        *landed += run.len();
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

/// The first seq of a run whose seqs are contiguous.
fn run_seq(seqs: &[Option<u64>]) -> Result<u64, FriendError> {
    let first = seqs.first().copied().flatten();
    let contiguous =
        first.is_some_and(|first| (first..).zip(seqs).all(|(want, have)| *have == Some(want)));
    match first {
        Some(first) if contiguous => Ok(first),
        _ => Err(FriendError::NotReady(format!(
            "chain append: the run's seqs {seqs:?} are not assigned in order"
        ))),
    }
}

async fn release(host: &dyn FriendHost, to: &str) -> Result<(), FriendError> {
    let (owner, peer) = (state::owner(host)?, to.to_string());
    let released = state::db_call!(host, "release unlanded chain seqs", move |conn| {
        chain_entries::release_unlanded(conn, &owner, &peer)
    })
    .await?;
    tracing::info!(to = %to, released, "dm chain hole: unlanded seqs released");
    Ok(())
}
