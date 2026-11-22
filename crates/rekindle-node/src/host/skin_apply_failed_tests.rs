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
