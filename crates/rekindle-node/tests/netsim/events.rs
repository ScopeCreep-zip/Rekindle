//! Community events (plan E3.1 E4): an event one member schedules reaches
//! another, whose RSVP both then list as an attendee; and the Host's
//! reminder fires ten minutes before the start on the simulated clock.

use std::time::Duration;

use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::event::{EventInfo, EventRsvp, NewEvent};
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};

use crate::harness::{eventually, Community, SimNode};

const LIMIT: Duration = Duration::from_secs(180);

fn new_event(title: &str, start_time: u64) -> NewEvent {
    NewEvent {
        title: title.into(),
        description: "bring potions".into(),
        start_time,
        end_time: None,
        channel_id: None,
        max_attendees: Some(8),
        cover_image_ref: None,
        recurrence: None,
        location: None,
    }
}

async fn create(node: &SimNode, community: &str, event: NewEvent) -> String {
    let id = node
        .ok(IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventCreate {
                community: community.into(),
                event,
            },
        )))
        .await;
    id.as_str().expect("the event's id").to_owned()
}

async fn events(node: &SimNode, community: &str) -> Vec<EventInfo> {
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventList {
                community: community.into(),
            },
        )))
        .await;
    serde_json::from_value(listed).expect("an event list")
}

async fn attendees(node: &SimNode, community: &str, event_id: &str) -> Vec<EventRsvp> {
    let listed = node
        .ok(IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventAttendees {
                community: community.into(),
                event_id: event_id.into(),
            },
        )))
        .await;
    serde_json::from_value(listed).expect("an attendee list")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn event_rsvp_converges() {
    let community = Community::of(2, 173).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    let key = community.governance_key.clone();

    let id = create(
        &community.nodes[0],
        &key,
        new_event("Raid night", 4_000_000_000),
    )
    .await;
    let seen = eventually(&community.sim, LIMIT, || async {
        events(&community.nodes[1], &key)
            .await
            .iter()
            .any(|e| e.id == id && e.title == "Raid night")
    })
    .await;
    assert!(seen, "the other member lists the event");

    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventRsvp {
                community: key.clone(),
                event_id: id.clone(),
                status: "going".into(),
            },
        )))
        .await;
    let member = community.pseudonym(1).await;
    let going = |list: &[EventRsvp]| {
        list.iter()
            .any(|r| r.pseudonym_key == member && r.status == "going")
    };
    assert!(
        going(&attendees(&community.nodes[1], &key, &id).await),
        "the answering member lists itself at once"
    );
    let converged = eventually(&community.sim, LIMIT, || async {
        going(&attendees(&community.nodes[0], &key, &id).await)
    })
    .await;
    assert!(converged, "the creator lists the other member as going");
    let listed = events(&community.nodes[0], &key).await;
    assert!(
        listed.iter().any(|e| e.id == id && going(&e.rsvps)),
        "the event list carries the attendee"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_event_reminder_fires_ten_minutes_before_its_start() {
    let community = Community::create(1, 181).await;
    let node = &community.nodes[0];
    let key = community.governance_key.clone();
    let stream = node.events().await;

    // The Host clock's now, as the first event's creation time.
    let asked = tokio::time::Instant::now();
    let first = create(node, &key, new_event("Later", 4_000_000_000)).await;
    let now = events(node, &key)
        .await
        .into_iter()
        .find(|e| e.id == first)
        .expect("the first event")
        .created_at;
    let id = create(node, &key, new_event("Soon", now + 15 * 60)).await;

    let reminded = eventually(&community.sim, Duration::from_secs(20 * 60), || async {
        stream.any(|e| {
            matches!(
                e,
                SubscriptionEvent::Social(SocialEvent::EventReminder { event_id, .. })
                    if *event_id == id
            )
        })
    })
    .await;
    assert!(reminded, "the Host reminds of the event");
    // The Host clock reads whole seconds and the node's skew.
    let at = asked.elapsed();
    assert!(
        at >= Duration::from_secs(5 * 60 - 2) && at < Duration::from_secs(6 * 60),
        "ten minutes before the start: {at:?}"
    );
    assert!(
        !stream.any(|e| matches!(
            e,
            SubscriptionEvent::Social(SocialEvent::EventReminder { event_id, .. }) if *event_id == first
        )),
        "the far event is not reminded"
    );
}
