//! The own-records stage is told to every frontend (plan E2.5 FC3): the
//! windows awaiting an invite, a friend request or a first message show
//! "Publishing your identity…" from `NetworkEvent::OwnRecordsChanged`,
//! sent at the unlock (`Pending`) and when the stage ends (`Held`, or
//! `Failed` with its reason).

use rekindle_transport::operations::friend::OwnRecordsStage;
use rekindle_types::subscription_events::{
    NetworkEvent, OwnRecordsPhase, StreamFrame, SubscriptionEvent,
};

use super::{create_ann, offline_host};

/// The stage events published since the last read.
async fn stages(
    stream: &mut rekindle_events::EventStream,
) -> Vec<(OwnRecordsPhase, Option<String>)> {
    let mut out = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(100), stream.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged { phase, error }) =
                s.event
            {
                out.push((phase, error));
            }
        }
    }
    out
}

#[tokio::test]
async fn the_stage_is_published_at_the_unlock_and_when_it_ends() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);

    create_ann(&host).await;
    assert_eq!(
        stages(&mut events).await,
        [(OwnRecordsPhase::Pending, None)],
        "the unlock tells the stage is under way"
    );
    assert_eq!(host.own_records_stage(), OwnRecordsStage::Pending);

    let stage = &host.ctx.friend_stores.own_records;
    stage.send_replace(OwnRecordsStage::Held);
    assert_eq!(stages(&mut events).await, [(OwnRecordsPhase::Held, None)]);

    stage.send_replace(OwnRecordsStage::Failed("no record pool".into()));
    assert_eq!(
        stages(&mut events).await,
        [(OwnRecordsPhase::Failed, Some("no record pool".into()))],
        "a failure carries its reason"
    );
    assert_eq!(
        host.own_records_stage(),
        OwnRecordsStage::Failed("no record pool".into())
    );
    host.shutdown().await.unwrap();
}
