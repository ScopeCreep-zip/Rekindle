//! The transfer driver's throttle and reveal (plan E7.2 G12, SK2-G design
//! §4): progress goes out at most four times a second, and Open reveals
//! only a finished regular file, never a link, and runs nothing.

use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};

use super::{revealable, Run, TransferCheckpoint, TransferRuns, PROGRESS_INTERVAL_MS};

const ATTACHMENT_ID: &str = "aa11";

fn running() -> TransferRuns {
    let runs = TransferRuns::default();
    let (abort, _registration) = futures::future::AbortHandle::new_pair();
    runs.0.lock().insert(
        ATTACHMENT_ID.to_owned(),
        Run {
            abort,
            seen: None,
            last_sent_ms: None,
        },
    );
    runs
}

fn at(chunks_done: u32) -> TransferCheckpoint {
    TransferCheckpoint {
        chunks_done,
        chunk_count: 1_000,
        bytes_done: u64::from(chunks_done) * 28_672,
        total: 28_672_000,
    }
}

#[test]
fn progress_is_capped_at_four_a_second() {
    let runs = running();
    // A chunk every 10 ms for two seconds.
    let sent: Vec<u64> = (0..200u32)
        .filter_map(|i| {
            let now = u64::from(i) * 10;
            runs.observe(ATTACHMENT_ID, at(i + 1), now).map(|_| now)
        })
        .collect();
    assert_eq!(sent.len(), 8, "four a second over two seconds: {sent:?}");
    assert!(
        sent.windows(2).all(|w| w[1] - w[0] >= PROGRESS_INTERVAL_MS),
        "no two within {PROGRESS_INTERVAL_MS} ms: {sent:?}"
    );
    // The unsent newest checkpoint is written when the download ends.
    assert_eq!(runs.end(ATTACHMENT_ID), Some(at(200)));
}

#[test]
fn the_last_chunk_is_always_published() {
    let runs = running();
    let whole = TransferCheckpoint {
        chunks_done: 1_000,
        ..at(0)
    };
    assert!(runs.observe(ATTACHMENT_ID, at(1), 0).is_some());
    assert_eq!(runs.observe(ATTACHMENT_ID, whole, 1), Some(whole));
    assert_eq!(
        runs.end(ATTACHMENT_ID),
        None,
        "the final checkpoint was written"
    );
    assert_eq!(
        runs.observe(ATTACHMENT_ID, at(2), 500),
        None,
        "no run, no progress"
    );
}

fn done_at(path: &std::path::Path) -> TransferRow {
    TransferRow {
        attachment_id: ATTACHMENT_ID.to_owned(),
        target: TransferTarget::Channel {
            community: "c".into(),
            channel_id: "ch".into(),
        },
        state: TransferState::Done,
        chunks_done: 1,
        chunk_count: 1,
        bytes_done: 1,
        total: 1,
        dest_name: "saved".into(),
        dest_path: path.display().to_string(),
        started_at: 0,
        finished_at: Some(1),
    }
}

#[cfg(unix)]
#[test]
fn open_refuses_a_link_and_runs_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a folder");
    let ran = dir.path().join("ran");
    let script = dir.path().join("saved.sh");
    std::fs::write(&script, format!("#!/bin/sh\ntouch '{}'\n", ran.display())).expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let link = dir.path().join("link.sh");
    std::os::unix::fs::symlink(&script, &link).expect("a link");

    assert_eq!(
        revealable(&done_at(&link)),
        None,
        "a link is never revealed"
    );
    assert_eq!(revealable(&done_at(dir.path())), None, "nor a folder");
    let paused = TransferRow {
        state: TransferState::Paused,
        ..done_at(&script)
    };
    assert_eq!(revealable(&paused), None, "nor an unfinished file");
    assert_eq!(
        revealable(&done_at(&script)),
        Some(script.display().to_string()),
        "a finished regular file is revealed by its path"
    );
    assert!(!ran.exists(), "revealing an executable never runs it");
}

/// Two checkpoints back to back (the resume's first, from the cache, and
/// the completing chunk's), then a read: it sees the newest, as the end
/// state and `TransferList` that follow the last chunk do.
#[tokio::test]
async fn a_read_after_the_last_checkpoint_sees_it() {
    let dir = tempfile::tempdir().expect("a folder");
    let (host, _) = crate::host::direct_tests::ann(&dir).await;
    let ctx = &host.ctx;
    let db = ctx.db.current().expect("unlocked");
    let owner = ctx.require_owner().expect("an owner");
    let row = TransferRow {
        state: TransferState::Active,
        chunks_done: 0,
        chunk_count: 0,
        bytes_done: 0,
        total: 0,
        finished_at: None,
        ..done_at(dir.path())
    };
    let o = owner.clone();
    db.call(move |c| rekindle_db::repo::transfers::begin(c, &o, &row))
        .await
        .expect("the row");
    let (abort, _registration) = futures::future::AbortHandle::new_pair();
    ctx.transfers.0.lock().insert(
        ATTACHMENT_ID.to_owned(),
        Run {
            abort,
            seen: None,
            last_sent_ms: None,
        },
    );
    let whole = at(1_000);
    super::chunk_cached(ctx, ATTACHMENT_ID, at(999));
    super::chunk_cached(ctx, ATTACHMENT_ID, whole);
    let stored = db
        .call(move |c| rekindle_db::repo::transfers::get(c, &owner, ATTACHMENT_ID))
        .await
        .expect("read")
        .expect("the row");
    assert_eq!(
        (stored.chunks_done, stored.bytes_done),
        (whole.chunks_done, whole.bytes_done),
        "the stored checkpoint is the newest cached"
    );
}
