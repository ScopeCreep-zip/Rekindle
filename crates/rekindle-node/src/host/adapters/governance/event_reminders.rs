//! Event reminders as a Host timer (plan E3.1 E4), one per unlock on both
//! modes. It sleeps until the soonest reminder among the scheduled events
//! of merged governance (on the Host clock, so the netsim drives it), then
//! journals `EventReminder` [`LEAD_SECS`] before each start; the shell's
//! OS notification follows from the journal (`notify_policy`). An event
//! created or moved later is planned at the next [`REPLAN`] at the latest;
//! a reminder whose time passed while the event is still ahead fires at
//! once with the minutes that remain.

use std::collections::HashSet;
use std::time::Duration;

use rekindle_types::event::EventStatus;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use super::community_events::event_key;
use super::HostGovernance;

/// How long before an event's start its reminder fires.
pub(crate) const LEAD_SECS: u64 = 10 * 60;
/// The longest sleep, so an event created meanwhile is planned.
const REPLAN: Duration = Duration::from_secs(30);

/// A scheduled event still ahead.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Upcoming {
    start_time: u64,
    community: String,
    event_id: String,
    title: String,
}

pub(super) async fn run(host: HostGovernance, stop: CancellationToken) {
    let mut fired: HashSet<(String, String, u64)> = HashSet::new();
    loop {
        let now = host.ctx.clock.now_secs();
        let mut next: Option<u64> = None;
        for event in host.upcoming_events(now) {
            let fire_at = event.start_time.saturating_sub(LEAD_SECS);
            if fire_at <= now {
                let key = (
                    event.community.clone(),
                    event.event_id.clone(),
                    event.start_time,
                );
                if fired.insert(key) {
                    host.remind(&event, now);
                }
            } else {
                next = Some(next.map_or(fire_at, |n| n.min(fire_at)));
            }
        }
        let wait = next.map_or(REPLAN, |at| Duration::from_secs(at - now).min(REPLAN));
        if stop
            .run_until_cancelled(tokio::time::sleep(wait))
            .await
            .is_none()
        {
            return;
        }
    }
}

impl HostGovernance {
    /// Every scheduled event of merged governance that starts after `now`,
    /// in start order.
    fn upcoming_events(&self, now: u64) -> Vec<Upcoming> {
        let map = self.ctx.communities.map().read();
        let mut out: Vec<Upcoming> = map
            .values()
            .filter_map(|c| c.governance_state.as_ref().map(|g| (c.id.clone(), g)))
            .flat_map(|(community, gov)| {
                gov.events
                    .iter()
                    .filter(|(_, e)| e.status == EventStatus::Scheduled && e.start_time > now)
                    .map(move |(id, e)| Upcoming {
                        start_time: e.start_time,
                        community: community.clone(),
                        event_id: event_key(id),
                        title: e.name.clone(),
                    })
            })
            .collect();
        out.sort();
        out
    }

    /// Journal `EventReminder` for `event`.
    fn remind(&self, event: &Upcoming, now: u64) {
        let minutes = (event.start_time.saturating_sub(now)).div_ceil(60);
        self.ctx
            .publish_event(SubscriptionEvent::Social(SocialEvent::EventReminder {
                community: event.community.clone(),
                event_id: event.event_id.clone(),
                title: event.title.clone(),
                minutes_until_start: u32::try_from(minutes).unwrap_or(u32::MAX),
            }));
    }
}
