//! "Keep this skin?" through `HostApi::call` on an offline host (plan
//! E7.7.2 "Apply"): a library skin chosen goes on trial, every window is
//! told, and the Host puts the stored choice back when the trial runs out
//! unless `SkinKeep` keeps it. The tests wait out the real
//! [`SKIN_TRIAL_MS`] in real time, as the Host's other timers are tested:
//! a paused tokio clock advances whenever the runtime waits on the
//! database thread, so it would fire the revert inside `SkinSelect`'s own
//! reply (`skin/mod.rs` `select` reads the library after `trial::begin`).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_ipc::{Caller, HostApi};
use rekindle_skin::{builtin, validate};
use rekindle_types::dto::skin_library::{
    SkinApplied, SkinEntry, SkinLibrary, SkinTrial, SKIN_TRIAL_MS,
};
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, SystemEvent};

use super::skin_requests::{data, run, stand_in};
use super::{create_ann, offline_host, status_of};
use crate::host::Host;

/// An unlocked host holding one library skin.
async fn host_with_library_skin(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = offline_host(dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(dir, &Ok(canonical.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: canonical.bytes,
        },
    )
    .await;
    let entry: SkinEntry = data(answer);
    (host, entry.id)
}

async fn library(host: &Host) -> SkinLibrary {
    data(host.call(Caller::shell(), IpcRequest::SkinList).await)
}

pub(super) async fn shown(host: &Host) -> String {
    let applied: SkinApplied = data(
        host.call(Caller::shell(), IpcRequest::SkinGet { id: None })
            .await,
    );
    applied.id
}

async fn select(host: &Host, id: &str) -> SkinLibrary {
    data(run(host, IpcRequest::SkinSelect { id: id.into() }).await.0)
}

/// The next `SkinTrialChanged` on the journal.
async fn next_trial(events: &mut rekindle_events::EventStream) -> Option<SkinTrial> {
    loop {
        let frame =
            tokio::time::timeout(Duration::from_millis(SKIN_TRIAL_MS + 10_000), events.next())
                .await
                .expect("SkinTrialChanged within the trial and 10 s")
                .expect("an event");
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::System(SystemEvent::SkinTrialChanged { trial }) = s.event {
                return trial;
            }
        }
    }
}

#[test]
fn a_trial_lasts_fifteen_seconds() {
    assert_eq!(SKIN_TRIAL_MS, 15_000);
}

#[tokio::test]
async fn a_library_skin_is_tried_then_reverted_unless_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (host, id) = host_with_library_skin(&dir).await;
    let before = library(&host).await.active;
    let mut events = host.ctx.journal.subscribe(None);
    let started = host.ctx.clock.now_ms();

    let chosen = select(&host, &id).await;
    let trial = chosen.trial.clone().expect("a library skin goes on trial");
    assert_eq!(trial.id, id);
    assert_eq!(trial.previous, before);
    assert!(
        (started + SKIN_TRIAL_MS..=started + SKIN_TRIAL_MS + 1_000).contains(&trial.reverts_at_ms)
    );
    assert_eq!(chosen.active, before, "not stored while on trial");
    assert_eq!(shown(&host).await, id, "every window shows the trial");
    assert_eq!(next_trial(&mut events).await, Some(trial));

    // Nobody keeps it: the Host reverts it.
    assert_eq!(next_trial(&mut events).await, None);
    let after = library(&host).await;
    assert!(after.trial.is_none());
    assert_eq!(after.active, before);
    assert_eq!(shown(&host).await, before);
    let late = run(
        &host,
        IpcRequest::SkinKeep {
            id: id.clone(),
            keep: true,
        },
    )
    .await
    .0;
    assert_eq!(status_of(&late), Some(409), "the trial is over");
}

#[tokio::test]
async fn a_kept_skin_is_stored_and_no_revert_follows() {
    let dir = tempfile::tempdir().unwrap();
    let (host, id) = host_with_library_skin(&dir).await;
    select(&host, &id).await;
    let kept: SkinLibrary = data(
        run(
            &host,
            IpcRequest::SkinKeep {
                id: id.clone(),
                keep: true,
            },
        )
        .await
        .0,
    );
    assert_eq!(kept.active, id);
    assert!(kept.trial.is_none());
    tokio::time::sleep(Duration::from_millis(SKIN_TRIAL_MS + 2_000)).await;
    assert_eq!(library(&host).await.active, id, "the old timer is void");
    assert_eq!(shown(&host).await, id);
}

#[tokio::test]
async fn revert_now_and_a_built_in_choice_end_the_trial() {
    let dir = tempfile::tempdir().unwrap();
    let (host, id) = host_with_library_skin(&dir).await;
    let before = library(&host).await.active;
    select(&host, &id).await;
    let removed = run(&host, IpcRequest::SkinRemove { id: id.clone() })
        .await
        .0;
    assert_eq!(status_of(&removed), Some(409), "the skin on trial stays");
    let reverted: SkinLibrary = data(
        run(
            &host,
            IpcRequest::SkinKeep {
                id: id.clone(),
                keep: false,
            },
        )
        .await
        .0,
    );
    assert!(reverted.trial.is_none());
    assert_eq!(shown(&host).await, before);

    select(&host, &id).await;
    let built_in = select(&host, builtin::DEFAULT).await;
    assert!(built_in.trial.is_none(), "a built-in is stored at once");
    assert_eq!(built_in.active, builtin::DEFAULT);
    tokio::time::sleep(Duration::from_millis(SKIN_TRIAL_MS + 2_000)).await;
    assert_eq!(shown(&host).await, builtin::DEFAULT);

    // A lock mid-trial leaves the stored choice.
    select(&host, &id).await;
    let owner = host.ctx.require_owner().unwrap();
    assert!(host.ctx.skin.trial.current(&owner).is_some());
    host.call(Caller::shell(), IpcRequest::Lock).await;
    assert!(host.ctx.skin.trial.current(&owner).is_none());
}
