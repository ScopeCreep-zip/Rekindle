//! Plan E7.2 G12 (SK2-G design §4): a channel attachment's download is a
//! transfer. A never-fetched attachment started without a destination
//! answers the shell's save dialog; a transfer stopped mid-way, or cut by
//! the node's restart, is `paused` with its checkpoint and Start resumes
//! it from the chunks already held to the attached bytes.

use std::time::Duration;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, PinRequest, TransferRequest};
use rekindle_types::dto::skin_run::{ShellOp, SkinRun};
use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};
use rekindle_types::subscription_events::{SubscriptionEvent, TransferEvent};

use crate::harness::{eventually, Community, Events, PASSPHRASE};

const LIMIT: Duration = Duration::from_secs(240);
const RESTART_DOWN: Duration = Duration::from_secs(30);

/// About twenty-two 28 KiB chunks, no two alike.
fn file_bytes() -> Vec<u8> {
    (0..600_000u32)
        .map(|i| u8::try_from((i.wrapping_mul(2_654_435_761) >> 24) & 0xFF).unwrap_or(0))
        .collect()
}

fn transfer(request: TransferRequest) -> IpcRequest {
    IpcRequest::Transfer(request)
}

/// Ann (node 0) attaches the file to general; its id and bea's target.
async fn attach(community: &Community, dir: &tempfile::TempDir) -> (String, TransferTarget) {
    let source = dir.path().join("notes.bin");
    std::fs::write(&source, file_bytes()).expect("the file to attach");
    let uploaded = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Pins(
            PinRequest::AttachmentUpload {
                community: community.anchor.clone(),
                channel_id: community.general_id(0).await,
                path: source.display().to_string(),
            },
        )))
        .await;
    let id = uploaded.as_str().expect("an attachment id").to_owned();
    let target = TransferTarget::Channel {
        community: community.anchor.clone(),
        channel_id: community.general_id(1).await,
    };
    (id, target)
}

fn start(id: &str, target: &TransferTarget, dest: Option<String>) -> IpcRequest {
    transfer(TransferRequest::TransferStart {
        attachment_id: id.to_owned(),
        target: target.clone(),
        dest,
    })
}

/// A progress event of `id` past `at_least` chunks and short of the end.
fn midway(events: &Events, id: &str, at_least: u32) -> bool {
    events.any(|e| {
        matches!(e, SubscriptionEvent::Transfer(TransferEvent::Progress {
            attachment_id, chunks_done, chunk_count, ..
        }) if attachment_id == id && *chunks_done >= at_least && chunks_done < chunk_count)
    })
}

/// Progress events of `id` below `chunks` (a fetch begun again from zero).
fn below(events: &Events, id: &str, chunks: u32) -> usize {
    events.matching(|e| {
        matches!(e, SubscriptionEvent::Transfer(TransferEvent::Progress {
            attachment_id, chunks_done, ..
        }) if attachment_id == id && *chunks_done < chunks)
    })
}

async fn row(community: &Community, i: usize, id: &str) -> TransferRow {
    let rows: Vec<TransferRow> = serde_json::from_value(
        community.nodes[i]
            .ok(transfer(TransferRequest::TransferList))
            .await,
    )
    .expect("transfer rows");
    rows.into_iter()
        .find(|r| r.attachment_id == id)
        .expect("the transfer is listed")
}

/// Bea's Start without a destination, once the offer is ingested, asks the
/// shell for one named after the offer.
async fn asks_for_a_destination(community: &Community, id: &str, target: &TransferTarget) {
    let asked = eventually(&community.sim, LIMIT, || async {
        let answer = community.nodes[1].call(start(id, target, None)).await;
        answer
            .ok()
            .and_then(|v| serde_json::from_value(v).ok())
            .is_some_and(|run: Option<SkinRun>| {
                matches!(run, Some(SkinRun::Shell(ShellOp::ChooseDownloadDest {
                    attachment_id, target: t, suggested_name,
                })) if attachment_id == id && &t == target && suggested_name == "notes.bin")
            })
    })
    .await;
    assert!(asked, "a never-fetched start answers ChooseDownloadDest");
}

/// Resume `id` from its paused checkpoint; it completes to the attached
/// bytes without fetching from zero again.
async fn resumes(community: &Community, id: &str, target: &TransferTarget, dest: &std::path::Path) {
    let paused = row(community, 1, id).await;
    assert_eq!(
        paused.state,
        TransferState::Paused,
        "the cut transfer is paused"
    );
    assert!(
        paused.chunks_done >= 2 && paused.chunks_done < paused.chunk_count,
        "the checkpoint is mid-way: {paused:?}"
    );
    let seen = community.nodes[1].events().await;
    community.nodes[1].ok(start(id, target, None)).await;
    assert_eq!(
        below(&seen, id, paused.chunks_done),
        0,
        "the resumed fetch starts from the checkpoint ({} chunks)",
        paused.chunks_done
    );
    let done = row(community, 1, id).await;
    assert_eq!(done.state, TransferState::Done);
    assert_eq!(done.chunks_done, done.chunk_count);
    assert_eq!(std::fs::read(dest).expect("the saved file"), file_bytes());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_stopped_transfer_resumes_from_its_checkpoint() {
    let community = Community::of(2, 241).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let dir = tempfile::tempdir().expect("a folder");
    let (id, target) = attach(&community, &dir).await;
    asks_for_a_destination(&community, &id, &target).await;

    let dest = dir.path().join("saved.bin");
    let bea = &community.nodes[1];
    let seen = bea.events().await;
    let run = bea.call(start(&id, &target, Some(dest.display().to_string())));
    let stop = async {
        for _ in 0..2_400 {
            if midway(&seen, &id, 2) {
                return bea
                    .call(transfer(TransferRequest::TransferStop {
                        attachment_id: id.clone(),
                    }))
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("no progress mid-way");
    };
    let (ran, stopped) = tokio::join!(run, stop);
    assert!(stopped.is_ok(), "the running transfer stops: {stopped:?}");
    assert!(
        ran.as_ref().is_err_and(|r| r.code == 409),
        "the stopped start answers 409: {ran:?}"
    );
    resumes(&community, &id, &target, &dest).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transfer_cut_by_a_restart_resumes_from_its_checkpoint() {
    let mut community = Community::of(2, 243).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let dir = tempfile::tempdir().expect("a folder");
    let (id, target) = attach(&community, &dir).await;
    asks_for_a_destination(&community, &id, &target).await;

    let dest = dir.path().join("saved.bin");
    {
        let bea = &community.nodes[1];
        let seen = bea.events().await;
        let run = bea.call(start(&id, &target, Some(dest.display().to_string())));
        let cut = async {
            for _ in 0..2_400 {
                if midway(&seen, &id, 2) {
                    // Past the next throttle window, so a checkpoint is written.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    if midway(&seen, &id, 3) {
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            panic!("no progress mid-way");
        };
        tokio::select! {
            ran = run => panic!("the download ended before the restart: {ran:?}"),
            () = cut => {}
        }
    }
    let key = community.nodes[1].public_key.clone().expect("an identity");
    let restarted = community
        .nodes
        .remove(1)
        .restart(&community.sim, RESTART_DOWN)
        .await;
    community.nodes.insert(1, restarted);
    community.nodes[1]
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    resumes(&community, &id, &target, &dest).await;
}
