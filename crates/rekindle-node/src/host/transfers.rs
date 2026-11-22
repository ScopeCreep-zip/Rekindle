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

/// The downloads running now, by attachment id.
#[derive(Debug, Default)]
pub struct TransferRuns(parking_lot::Mutex<BTreeMap<String, tokio::task::AbortHandle>>);

impl TransferRuns {
    /// Abort the running download of `attachment_id`; `false` when none runs.
    pub(crate) fn stop(&self, attachment_id: &str) -> bool {
        self.0
            .lock()
            .remove(attachment_id)
            .inspect(tokio::task::AbortHandle::abort)
            .is_some()
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
    let task = tokio::spawn(async move {
        files
            .download(&community, &channel_id, &id, Path::new(&dest))
            .await
    });
    ctx.transfers
        .0
        .lock()
        .insert(attachment_id.to_owned(), task.abort_handle());
    let ended = match task.await {
        Ok(Ok(())) => Ended::Done,
        Ok(Err(e)) => Ended::Failed(e),
        Err(_) => Ended::Stopped,
    };
    if !matches!(ended, Ended::Stopped) {
        ctx.transfers.0.lock().remove(attachment_id);
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
        // `stop` wrote `paused` (or `remove` the row's end).
        Ended::Stopped => {}
    }
    Ok(ended)
}

/// Pause the running download: abort its task and mark it `paused`.
/// `false` when none runs.
pub(crate) async fn stop(ctx: &DaemonContext, attachment_id: &str) -> bool {
    if !ctx.transfers.stop(attachment_id) {
        return false;
    }
    set_state(ctx, attachment_id, TransferState::Paused).await;
    true
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
