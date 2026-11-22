//! `SkinApplyFailed` through `HostApi::call` (plan SK2-T1): a window
//! reports a skin it could not apply; the request is validated, the Host
//! logs it at WARN, tells that window and Settings
//! (`SystemEvent::SkinApplyFailed`) and ends a trial of that skin at once.

use std::time::Duration;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_skin::{builtin, validate};
use rekindle_types::dto::skin_library::{SkinApplyFailure, SkinEntry, SkinLibrary};
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, SystemEvent};

use super::skin_requests::{data, run, stand_in};
use super::skin_trial::shown;
use super::{create_ann, offline_host, status_of};
use crate::host::direct_call_tests::{capture, Captured};
use crate::host::Host;

// No 16-hex run, which the log scrubber would cut.
const REPORTING_WINDOW: &str = "chat-ann";

fn image() -> SkinApplyFailure {
    SkinApplyFailure::ImageDecode {
        path: "skin://pkg/frame/top.png".into(),
    }
}

fn failed(window: &str, skin: &str, cause: SkinApplyFailure) -> IpcRequest {
    IpcRequest::SkinApplyFailed {
        window: window.into(),
        skin: skin.into(),
        cause,
    }
}

async fn report(host: &Host, request: IpcRequest) -> IpcResponse {
    host.call(Caller::shell(), request).await
}

async fn library(host: &Host) -> SkinLibrary {
    data(host.call(Caller::shell(), IpcRequest::SkinList).await)
}

/// The next `SkinApplyFailed` on the journal, as (window, skin, cause, reverted).
async fn next_failed(
    events: &mut rekindle_events::EventStream,
) -> (String, String, SkinApplyFailure, bool) {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("SkinApplyFailed within 10 s")
            .expect("an event");
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::System(SystemEvent::SkinApplyFailed {
                window,
                skin,
                cause,
                reverted,
            }) = s.event
            {
                return (window, skin, cause, reverted);
            }
        }
    }
}

#[tokio::test]
async fn a_report_out_of_bounds_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let long = "x".repeat(65);
    for request in [
        failed("", builtin::DEFAULT, image()),
        failed(&long, builtin::DEFAULT, image()),
        failed(REPORTING_WINDOW, "", image()),
        failed(REPORTING_WINDOW, &"p".repeat(129), image()),
        failed(
            REPORTING_WINDOW,
            builtin::DEFAULT,
            SkinApplyFailure::ImageDecode {
                path: String::new(),
            },
        ),
        failed(
            REPORTING_WINDOW,
            builtin::DEFAULT,
            SkinApplyFailure::SheetError {
                detail: "d".repeat(1025),
            },
        ),
    ] {
        assert_eq!(
            status_of(&report(&host, request.clone()).await),
            Some(400),
            "{request:?}"
        );
    }
}

/// Before unlock (the login window's device skin) and with no trial, the
/// report is logged and told, and nothing is reverted.
#[tokio::test]
async fn a_report_without_a_trial_is_told_and_reverts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let sheet = SkinApplyFailure::SheetError {
        detail: "the stylesheet failed to load".into(),
    };
    let locked = report(&host, failed("login", builtin::DEFAULT, sheet.clone())).await;
    assert_eq!(
        status_of(&locked),
        None,
        "answered before unlock: {locked:?}"
    );
    assert_eq!(
        next_failed(&mut events).await,
        ("login".into(), builtin::DEFAULT.into(), sheet, false)
    );

    create_ann(&host).await;
    let before = library(&host).await.active;
    let mut events = host.ctx.journal.subscribe(None);
    let unlocked = report(&host, failed(REPORTING_WINDOW, builtin::DEFAULT, image())).await;
    assert_eq!(status_of(&unlocked), None, "{unlocked:?}");
    assert_eq!(
        next_failed(&mut events).await,
        (
            REPORTING_WINDOW.into(),
            builtin::DEFAULT.into(),
            image(),
            false
        )
    );
    assert_eq!(library(&host).await.active, before);
}

/// A library skin on trial that a window could not apply is reverted at
/// once, the stored choice back in every window, and the failure is
/// logged at WARN.
#[tokio::test]
async fn a_skin_on_trial_that_fails_is_reverted_and_logged() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Ok(canonical.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: canonical.bytes,
        },
    )
    .await;
    let id = data::<SkinEntry>(answer).id;
    let before = library(&host).await.active;
    let chosen: SkinLibrary = data(
        run(&host, IpcRequest::SkinSelect { id: id.clone() })
            .await
            .0,
    );
    assert!(chosen.trial.is_some(), "a library skin goes on trial");
    assert_eq!(shown(&host).await, id);

    let mut events = host.ctx.journal.subscribe(None);
    let captured = Captured::default();
    let answer = {
        let _armed = capture(&captured);
        report(&host, failed(REPORTING_WINDOW, &id, image())).await
    };
    assert_eq!(status_of(&answer), None, "{answer:?}");
    assert_eq!(
        next_failed(&mut events).await,
        (REPORTING_WINDOW.into(), id.clone(), image(), true)
    );
    let after = library(&host).await;
    assert!(after.trial.is_none(), "the trial ended");
    assert_eq!(after.active, before);
    assert_eq!(shown(&host).await, before, "the stored choice is back");
    let log = String::from_utf8(captured.0.lock().clone()).unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("a window could not apply a skin"))
        .unwrap_or_else(|| panic!("no WARN line in {log}"));
    assert!(line.contains("WARN"), "{line}");
    assert!(line.contains(REPORTING_WINDOW), "{line}");
}

const WINDOWS: [&str; 3] = ["chat-ann", "buddy-list", "settings"];

fn sheet() -> SkinApplyFailure {
    SkinApplyFailure::SheetError {
        detail: "the stylesheet failed to load".into(),
    }
}

/// Every window applying a skin reports its own failure: N reports of one
/// (skin, cause) are one event, logged at WARN once. A different cause is
/// a new event, and so is the same cause after the skin is applied again.
#[tokio::test]
async fn n_windows_reporting_one_failure_publish_one_event() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Ok(canonical)))
        .unwrap();
    let mut events = host.ctx.journal.subscribe(None);
    let captured = Captured::default();
    {
        let _armed = capture(&captured);
        for window in WINDOWS {
            let answer = report(&host, failed(window, builtin::DEFAULT, image())).await;
            assert_eq!(status_of(&answer), None, "{answer:?}");
        }
    }
    // Events are in order: the next one after the first is the other cause.
    let _ = report(&host, failed(WINDOWS[1], builtin::DEFAULT, sheet())).await;
    assert_eq!(
        next_failed(&mut events).await,
        (WINDOWS[0].into(), builtin::DEFAULT.into(), image(), false)
    );
    assert_eq!(
        next_failed(&mut events).await,
        (WINDOWS[1].into(), builtin::DEFAULT.into(), sheet(), false),
        "the other windows' reports of the same failure published nothing"
    );
    let log = String::from_utf8(captured.0.lock().clone()).unwrap();
    let warned = log
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("could not apply a skin"))
        .count();
    assert_eq!(warned, 1, "WARN once, not once per window: {log}");

    // The user applies the skin again: the same failure is told again.
    let again: SkinLibrary = data(
        run(
            &host,
            IpcRequest::SkinSelect {
                id: builtin::DEFAULT.into(),
            },
        )
        .await
        .0,
    );
    assert_eq!(again.active, builtin::DEFAULT);
    for window in WINDOWS {
        let _ = report(&host, failed(window, builtin::DEFAULT, image())).await;
    }
    let _ = report(&host, failed(WINDOWS[2], builtin::DEFAULT, sheet())).await;
    assert_eq!(
        next_failed(&mut events).await,
        (WINDOWS[0].into(), builtin::DEFAULT.into(), image(), false),
        "a re-apply tells the same failure again"
    );
    assert_eq!(
        next_failed(&mut events).await,
        (WINDOWS[2].into(), builtin::DEFAULT.into(), sheet(), false)
    );
}

/// N windows failing a skin on trial: one event, the trial reverted once,
/// and the late reports leave the stored choice shown. Trying the skin
/// again tells its failure again.
#[tokio::test]
async fn n_windows_failing_a_trial_revert_it_once_and_tell_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let canonical = validate::run(builtin::PACKS[0].bytes).unwrap();
    host.ctx
        .skin
        .helper
        .set(stand_in(&dir, &Ok(canonical.clone())))
        .unwrap();
    let (answer, _) = run(
        &host,
        IpcRequest::SkinImport {
            bytes: canonical.bytes,
        },
    )
    .await;
    let id = data::<SkinEntry>(answer).id;
    let before = library(&host).await.active;
    let select = IpcRequest::SkinSelect { id: id.clone() };
    for round in 0..2 {
        let chosen: SkinLibrary = data(run(&host, select.clone()).await.0);
        assert!(chosen.trial.is_some(), "round {round}: on trial");
        let mut events = host.ctx.journal.subscribe(None);
        for window in WINDOWS {
            let answer = report(&host, failed(window, &id, image())).await;
            assert_eq!(status_of(&answer), None, "{answer:?}");
        }
        let _ = report(&host, failed(WINDOWS[1], &id, sheet())).await;
        assert_eq!(
            next_failed(&mut events).await,
            (WINDOWS[0].into(), id.clone(), image(), true),
            "round {round}"
        );
        assert_eq!(
            next_failed(&mut events).await,
            (WINDOWS[1].into(), id.clone(), sheet(), false),
            "round {round}: one event for the three reports"
        );
        let after = library(&host).await;
        assert!(after.trial.is_none(), "the trial ended");
        assert_eq!(shown(&host).await, before, "the stored choice is back");
    }
}
