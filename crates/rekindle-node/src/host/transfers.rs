//! The Host's file transfers (plan E7.2 G12, SK2-G design §4): each
//! download runs on a task of its own, recorded in `transfers`; Stop
//! aborts the task and leaves the row `paused`. The held chunks stay in
//! the chunk cache, so a resumed fetch (`fetch_attachment_to_cache`,
//! which starts from the cache's bitmap) asks peers only for the rest.
//! Every state change is published as a `TransferEvent`; the skin
//! capabilities and the Downloads view fold it (lead decision 51).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rekindle_files::FilesError;
use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};
use rekindle_types::subscription_events::{SubscriptionEvent, TransferEvent};

use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::files::HostFiles;

/// The least time between two progress events (and checkpoint writes)
/// of one transfer: at most four a second (SK2-G design §4).
pub(crate) const PROGRESS_INTERVAL_MS: u64 = 250;

/// One running download: how to stop it and the last checkpoint seen.
#[derive(Debug)]
struct Run {
    abort: futures::future::AbortHandle,
    /// The newest `ChunkCached` counts, and whether they are written.
    seen: Option<(Checkpoint, bool)>,
    /// When the last progress event went out (Host clock, ms).
    last_sent_ms: Option<u64>,
}

/// A transfer's checkpoint: chunks held of `chunk_count`, bytes of `total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    pub chunks_done: u32,
    pub chunk_count: u32,
    pub bytes_done: u64,
    pub total: u64,
}

/// The downloads running now, by attachment id.
#[derive(Debug, Default)]
pub struct TransferRuns(parking_lot::Mutex<BTreeMap<String, Run>>);

impl TransferRuns {
    /// Abort the running download of `attachment_id`; `false` when none runs.
    pub(crate) fn stop(&self, attachment_id: &str) -> bool {
        self.0
            .lock()
            .get(attachment_id)
            .inspect(|run| run.abort.abort())
            .is_some()
    }

    /// Record `at` for a running download; the checkpoint to write and the
    /// progress to publish when the last went out at least
    /// [`PROGRESS_INTERVAL_MS`] ago or the download is complete. `None`
    /// for an attachment no transfer runs (a voice message, an expression).
    fn observe(&self, attachment_id: &str, at: Checkpoint, now_ms: u64) -> Option<Checkpoint> {
        let mut runs = self.0.lock();
        let run = runs.get_mut(attachment_id)?;
        let due = at.chunks_done == at.chunk_count
            || run
                .last_sent_ms
                .is_none_or(|last| now_ms.saturating_sub(last) >= PROGRESS_INTERVAL_MS);
        run.seen = Some((at, due));
        if due {
            run.last_sent_ms = Some(now_ms);
        }
        due.then_some(at)
    }

    /// Forget a download that ended: its last checkpoint when unwritten.
    fn end(&self, attachment_id: &str) -> Option<Checkpoint> {
        let run = self.0.lock().remove(attachment_id)?;
        run.seen.and_then(|(at, written)| (!written).then_some(at))
    }
}

/// `FilesEvent::ChunkCached` (the one subscriber, lead decision 51): a
/// running transfer's checkpoint is written and its progress published,
/// throttled to [`PROGRESS_INTERVAL_MS`].
pub(crate) fn chunk_cached(ctx: &DaemonContext, attachment_id: &str, at: Checkpoint) {
    let now = u64::try_from(ctx.clock.now_ms_i64()).unwrap_or(0);
    let Some(at) = ctx.transfers.observe(attachment_id, at, now) else {
        return;
    };
    publish(
        ctx,
        TransferEvent::Progress {
            attachment_id: attachment_id.to_owned(),
            chunks_done: at.chunks_done,
            chunk_count: at.chunk_count,
            bytes_done: at.bytes_done,
            total: at.total,
        },
    );
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return;
    };
    let id = attachment_id.to_owned();
    tokio::spawn(checkpoint_into(db, owner, id, at));
}

async fn write_checkpoint(ctx: &DaemonContext, attachment_id: &str, at: Checkpoint) {
    if let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) {
        checkpoint_into(db, owner, attachment_id.to_owned(), at).await;
    }
}

async fn checkpoint_into(db: rekindle_db::Db, owner: String, id: String, at: Checkpoint) {
    let wrote = db
        .call(move |conn| {
            rekindle_db::repo::transfers::checkpoint(
                conn,
                &owner,
                &id,
                (at.chunks_done, at.chunk_count),
                (at.bytes_done, at.total),
            )
        })
        .await;
    if let Err(e) = wrote {
        tracing::warn!(error = %e, "transfer checkpoint unwritten");
    }
}

/// At unlock: a transfer the last Host left running is `paused`, so Start
/// resumes it from its checkpoint (design §4 (3)).
pub(crate) async fn pause_interrupted(ctx: &DaemonContext) {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return;
    };
    let paused = db
        .call(move |conn| rekindle_db::repo::transfers::pause_interrupted(conn, &owner))
        .await;
    match paused {
        Ok(0) => {}
        Ok(n) => tracing::info!(paused = n, "interrupted transfers paused"),
        Err(e) => tracing::warn!(error = %e, "interrupted transfers unread"),
    }
}

/// How a download ended.
#[derive(Debug)]
pub(crate) enum Ended {
    Done,
    /// Stopped by `TransferStop` or `TransferRemove`.
    Stopped,
    Failed(FilesError),
}

/// What a transfer step failed on, before any download ran.
#[derive(Debug)]
pub(crate) enum Refused {
    Locked,
    Db(String),
}

fn publish(ctx: &DaemonContext, event: TransferEvent) {
    ctx.publish_event(SubscriptionEvent::Transfer(event));
}

async fn set_state(ctx: &DaemonContext, attachment_id: &str, state: TransferState) {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return;
    };
    let now = ctx.clock.now_ms_i64();
    let id = attachment_id.to_owned();
    let wrote = db
        .call(move |conn| rekindle_db::repo::transfers::set_state(conn, &owner, &id, state, now))
        .await;
    if let Err(e) = wrote {
        tracing::warn!(error = %e, "transfer state unwritten");
    }
    publish(
        ctx,
        TransferEvent::StateChanged {
            attachment_id: attachment_id.to_owned(),
            state,
        },
    );
}

/// Download `attachment_id` of `target` to `dest`, recorded as a transfer
/// (a row already present keeps its checkpoint), and wait for it to end.
///
/// # Errors
/// [`Refused`] when no identity is unlocked or the row is unwritten.
pub(crate) async fn download(
    ctx: &DaemonContext,
    target: TransferTarget,
    attachment_id: &str,
    dest: PathBuf,
) -> Result<Ended, Refused> {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Err(Refused::Locked);
    };
    let TransferTarget::Channel {
        community,
        channel_id,
    } = target.clone();
    let row = TransferRow {
        attachment_id: attachment_id.to_owned(),
        target,
        state: TransferState::Active,
        chunks_done: 0,
        chunk_count: 0,
        bytes_done: 0,
        total: 0,
        dest_name: dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dest_path: dest.to_string_lossy().into_owned(),
        started_at: ctx.clock.now_ms_i64(),
        finished_at: None,
    };
    db.call(move |conn| rekindle_db::repo::transfers::begin(conn, &owner, &row))
        .await
        .map_err(|e| Refused::Db(e.to_string()))?;
    publish(
        ctx,
        TransferEvent::StateChanged {
            attachment_id: attachment_id.to_owned(),
            state: TransferState::Active,
        },
    );
    let files = HostFiles::new(crate::daemon::dispatch::adapter(ctx));
    let id = attachment_id.to_owned();
    // Registered before the fetch starts, so its first chunk is seen.
    let (abort, registration) = futures::future::AbortHandle::new_pair();
    ctx.transfers.0.lock().insert(
        attachment_id.to_owned(),
        Run {
            abort,
            seen: None,
            last_sent_ms: None,
        },
    );
    let fetch = futures::future::Abortable::new(
        async move {
            files
                .download(&community, &channel_id, &id, Path::new(&dest))
                .await
        },
        registration,
    );
    let ended = match fetch.await {
        Ok(Ok(())) => Ended::Done,
        Ok(Err(e)) => Ended::Failed(e),
        Err(futures::future::Aborted) => Ended::Stopped,
    };
    if let Some(at) = ctx.transfers.end(attachment_id) {
        write_checkpoint(ctx, attachment_id, at).await;
    }
    match &ended {
        Ended::Done => {
            set_state(ctx, attachment_id, TransferState::Done).await;
            publish(
                ctx,
                TransferEvent::DownloadComplete {
                    attachment_id: attachment_id.to_owned(),
                },
            );
        }
        Ended::Failed(_) => set_state(ctx, attachment_id, TransferState::Failed).await,
        // Paused by `stop` (or forgotten by `remove`, which wrote the end).
        Ended::Stopped => {
            if get(ctx, attachment_id).await.is_ok_and(|row| row.is_some()) {
                set_state(ctx, attachment_id, TransferState::Paused).await;
            }
        }
    }
    Ok(ended)
}

/// Pause the running download: abort it; its `download` writes the last
/// checkpoint and marks it `paused`. `false` when none runs.
pub(crate) fn stop(ctx: &DaemonContext, attachment_id: &str) -> bool {
    ctx.transfers.stop(attachment_id)
}

/// Forget a transfer, stopping it first; with `delete_file`, delete the
/// regular file at its stored path (never a directory or a link).
pub(crate) async fn remove(
    ctx: &DaemonContext,
    row: &TransferRow,
    delete_file: bool,
) -> Result<(), Refused> {
    ctx.transfers.stop(&row.attachment_id);
    if delete_file {
        let path = Path::new(&row.dest_path);
        if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
            std::fs::remove_file(path).map_err(|e| Refused::Db(format!("delete file: {e}")))?;
        }
    }
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Err(Refused::Locked);
    };
    let id = row.attachment_id.clone();
    db.call(move |conn| rekindle_db::repo::transfers::remove(conn, &owner, &id))
        .await
        .map_err(|e| Refused::Db(e.to_string()))?;
    publish(
        ctx,
        TransferEvent::Removed {
            attachment_id: row.attachment_id.clone(),
        },
    );
    Ok(())
}

/// The stored transfer `attachment_id`.
pub(crate) async fn get(
    ctx: &DaemonContext,
    attachment_id: &str,
) -> Result<Option<TransferRow>, Refused> {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Err(Refused::Locked);
    };
    let id = attachment_id.to_owned();
    db.call(move |conn| rekindle_db::repo::transfers::get(conn, &owner, &id))
        .await
        .map_err(|e| Refused::Db(e.to_string()))
}

/// Every transfer, newest first.
pub(crate) async fn list(ctx: &DaemonContext) -> Result<Vec<TransferRow>, Refused> {
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Err(Refused::Locked);
    };
    db.call(move |conn| rekindle_db::repo::transfers::list(conn, &owner))
        .await
        .map_err(|e| Refused::Db(e.to_string()))
}

/// The finished file `row` saved, to reveal: `None` unless it is `done`
/// and a regular file (not a link or a directory) is at its path.
pub(crate) fn revealable(row: &TransferRow) -> Option<String> {
    (row.state == TransferState::Done
        && std::fs::symlink_metadata(&row.dest_path).is_ok_and(|m| m.is_file()))
    .then(|| row.dest_path.clone())
}
