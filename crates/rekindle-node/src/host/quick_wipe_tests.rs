//! The quick wipe's state reaches every frontend when its intent commits
//! (plan E7.1): the GUI registers the panic shortcut on
//! `QuickWipeChanged`, so a chord armed through the U1 review is live at
//! once, and a disarmed one is released at once, never at the next unlock
//! or lock.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::intent::{IntentClass, IntentProof};
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, SystemEvent};

use super::{create_ann, offline_host};
use crate::host::Host;

/// Send `request`, which the gate holds as Protected, and commit it with
/// Ann's passphrase.
async fn commit_protected(host: &Host, request: IpcRequest) -> IpcResponse {
    let review = match host.call(Caller::shell(), request).await {
        IpcResponse::IntentRequired(review) => review,
        other => panic!("expected a review, got {other:?}"),
    };
    assert_eq!(review.class, IntentClass::Protected);
    let commit = IpcRequest::IntentCommit {
        intent_id: review.intent_id,
        proof: IntentProof::Passphrase(rekindle_test_fixtures::PASSPHRASE.to_string().into()),
    };
    host.call(Caller::shell(), commit).await
}

/// The `QuickWipeChanged` events the stream holds now.
async fn quick_wipe_changes(
    stream: &mut rekindle_events::EventStream,
) -> Vec<(bool, Option<String>)> {
    let mut changes = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
    {
        if let StreamFrame::Event(sequenced) = frame {
            if let SubscriptionEvent::System(SystemEvent::QuickWipeChanged { armed, shortcut }) =
                sequenced.event
            {
                changes.push((armed, shortcut));
            }
        }
    }
    changes
}

#[tokio::test]
async fn committing_arm_and_disarm_publishes_the_quick_wipe_state() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let mut events = host.ctx.journal.subscribe(None);

    let arm = IpcRequest::QuickWipeArm {
        duress_passphrase: "duress".to_string().into(),
        shortcut: Some("CommandOrControl+Shift+F12".into()),
    };
    let armed = commit_protected(&host, arm).await;
    assert!(matches!(armed, IpcResponse::Ok { .. }), "{armed:?}");
    assert_eq!(
        quick_wipe_changes(&mut events).await,
        vec![(true, Some("CommandOrControl+Shift+F12".to_string()))]
    );

    let disarmed = commit_protected(&host, IpcRequest::QuickWipeDisarm).await;
    assert!(matches!(disarmed, IpcResponse::Ok { .. }), "{disarmed:?}");
    assert_eq!(quick_wipe_changes(&mut events).await, vec![(false, None)]);
}
