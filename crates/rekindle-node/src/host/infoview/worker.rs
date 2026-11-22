//! The Infoview's push: one subscriber folding the Host's journal (lead
//! decision (51)). An event an open view shows marks the views stale; one
//! coalescing period later each open window gets its view once
//! (`SystemEvent::Infoview(InfoviewEvent::Updated)`, design §2.2: at most
//! one per window per 250 ms, not polled, D4).

use std::sync::Arc;
use std::time::Duration;

use rekindle_types::dto::infoview::{InfoviewEvent, InfoviewSubject};
use rekindle_types::subscription_events::{
    PresenceEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};
use tokio::time::Instant;

use crate::daemon::dispatch::DaemonContext;
use crate::host::coalesce::sleep_until;

/// How long the worker gathers events before it composes again.
pub(super) const COALESCE: Duration = Duration::from_millis(250);

/// Whether `event` changes what some Infoview shows: a friend's presence,
/// name, bio, group or friendship, a member's beat (the online count), a
/// feed entry or our favourite, an unread count, a membership. Never the Infoview's own push.
fn shown(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::Presence(
            PresenceEvent::FriendChanged { .. } | PresenceEvent::CommunityMemberChanged { .. }
        ) | SubscriptionEvent::Friend(_)
            | SubscriptionEvent::Membership(_)
            | SubscriptionEvent::UnreadChanged { .. }
            | SubscriptionEvent::System(
                SystemEvent::ActivityAppended { .. } | SystemEvent::FavouriteServerAdded { .. }
            )
    )
}

/// The worker: fold the Host's journal until shutdown. A shown event
/// pushes every open view; a selection change (`OpenViews::selection_changed`)
/// pushes only the views that follow that window's selection (plan E7.15,
/// IV5). A session's end (`InvalidSession`) closes every view.
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    let mut due: Option<Instant> = None;
    let mut all = false;
    loop {
        let frame = tokio::select! {
            () = ctx.shutdown.requested() => return,
            () = sleep_until(due), if due.is_some() => {
                due = None;
                push(&ctx, std::mem::take(&mut all)).await;
                continue;
            }
            () = ctx.infoview.changed.notified() => {
                due.get_or_insert_with(|| Instant::now() + COALESCE);
                continue;
            }
            frame = stream.next() => frame,
        };
        match frame {
            Some(StreamFrame::Event(s)) if shown(&s.event) => {
                all = true;
                due.get_or_insert_with(|| Instant::now() + COALESCE);
            }
            Some(StreamFrame::InvalidSession) => {
                (due, all) = (None, false);
                ctx.infoview.close_all();
            }
            Some(_) => {}
            None => return,
        }
    }
}

/// Compose the open windows' views again and push each to its window:
/// every view when `all`, else the views asking `Selection` whose launcher's
/// selection changed.
async fn push(ctx: &DaemonContext, all: bool) {
    let changed = ctx.infoview.take_changed();
    for (window, asked) in ctx.infoview.open() {
        let follows =
            asked == InfoviewSubject::Selection && changed.contains(&super::launcher(ctx, &window));
        if !(all || follows) {
            continue;
        }
        let subject = super::resolved(ctx, &window, &asked);
        match super::compose::view(ctx, &subject).await {
            Ok(view) => ctx.publish_event(SubscriptionEvent::System(SystemEvent::Infoview(
                InfoviewEvent::Updated { window, view },
            ))),
            Err(e) => tracing::debug!(%window, ?e, "infoview not composed"),
        }
    }
}
