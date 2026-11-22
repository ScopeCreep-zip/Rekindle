//! The Servers view's push: one subscriber folding the Host's journal (lead
//! decision (51)), like the Infoview's. An event the view shows marks it
//! stale; one coalescing period later the view goes out once
//! (`SystemEvent::ServersViewChanged`, at most one per 250 ms, not polled,
//! D4). Our own presence naming a server is a visit (`recent_servers`), a
//! friend's game invite is held for the Friends category.

use std::sync::Arc;
use std::time::Duration;

use rekindle_types::game_server::ServerAddress;
use rekindle_types::subscription_events::{
    FriendEvent, GameActivity, PresenceEvent, SocialEvent, StreamFrame, SubscriptionEvent,
    SystemEvent,
};
use tokio::time::Instant;

use super::HeldInvite;
use crate::daemon::dispatch::DaemonContext;
use crate::host::coalesce::sleep_until;

/// How long the worker gathers events before it composes again.
pub(super) const COALESCE: Duration = Duration::from_millis(250);

/// Whether `event` changes what the view shows: a friend's presence or
/// friendship (game invites included), a membership, a community's server
/// list, a favourite added. Never the view's own push. A favourite removed
/// has no journal event: its route marks the view stale
/// (`ServersState::mark_stale`).
fn shown(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged { .. })
            | SubscriptionEvent::Friend(_)
            | SubscriptionEvent::Membership(_)
            | SubscriptionEvent::Social(
                SocialEvent::GameServerAdded { .. } | SocialEvent::GameServerRemoved { .. }
            )
            | SubscriptionEvent::System(SystemEvent::FavouriteServerAdded { .. })
    )
}

/// Our own presence's game and server, when it names a parsed server.
fn own_visit(event: &SubscriptionEvent) -> Option<(u32, ServerAddress)> {
    let SubscriptionEvent::Presence(PresenceEvent::SelfChanged { snapshot, .. }) = event else {
        return None;
    };
    let Some(GameActivity::Playing {
        game_id: Some(game_id),
        server_address: Some(server),
        ..
    }) = &snapshot.game
    else {
        return None;
    };
    ServerAddress::parse(server).ok().map(|s| (*game_id, s))
}

/// The worker: fold the Host's journal until shutdown. A session's end
/// (`InvalidSession`) drops the held invites.
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    let mut due: Option<Instant> = None;
    loop {
        let frame = tokio::select! {
            () = ctx.shutdown.requested() => return,
            () = sleep_until(due), if due.is_some() => {
                due = None;
                push(&ctx).await;
                continue;
            }
            () = ctx.servers.marked_stale() => {
                due.get_or_insert_with(|| Instant::now() + COALESCE);
                continue;
            }
            frame = stream.next() => frame,
        };
        match frame {
            Some(StreamFrame::Event(s)) => {
                if let SubscriptionEvent::Friend(FriendEvent::GameInvite {
                    peer_key,
                    game_id,
                    server,
                }) = &s.event
                {
                    ctx.servers.hold(HeldInvite {
                        peer_key: peer_key.clone(),
                        game_id: *game_id,
                        server: server.as_str().to_owned(),
                    });
                }
                if let Some((game_id, server)) = own_visit(&s.event) {
                    super::visited(&ctx, game_id, &server).await;
                } else if shown(&s.event) {
                    due.get_or_insert_with(|| Instant::now() + COALESCE);
                }
            }
            Some(StreamFrame::InvalidSession) => {
                due = None;
                ctx.servers.clear();
            }
            Some(_) => {}
            None => return,
        }
    }
}

async fn push(ctx: &DaemonContext) {
    match super::compose(ctx).await {
        Ok(view) => ctx.publish_event(SubscriptionEvent::System(SystemEvent::ServersViewChanged {
            view,
        })),
        Err(e) => tracing::debug!(?e, "servers view not composed"),
    }
}
