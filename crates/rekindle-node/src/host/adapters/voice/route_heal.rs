//! Keep the live direct call's audio on the route held by the peer registry.

use std::sync::Arc;

use rekindle_lifecycle::{ScopeClosed, SessionScope};
use tokio::sync::broadcast;

use crate::daemon::dispatch::DaemonContext;

pub(crate) fn spawn_route_heal(
    ctx: &DaemonContext,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    let Some(node) = ctx.transport.read().clone() else {
        return Ok(());
    };
    let mut updates = node.peers().read().updates();
    let mut media = node
        .routes()
        .own_state(rekindle_records::port::RouteClass::Media);
    media.borrow_and_update();
    let ctx_route = ctx.this.clone();
    scope.spawn_with_token("voice route reannounce", move |stop| async move {
        while let Some(Ok(())) = stop.run_until_cancelled(media.changed()).await {
            if matches!(
                *media.borrow_and_update(),
                rekindle_records::port::RouteState::Available { .. }
            ) {
                let Some(ctx) = ctx_route.upgrade() else {
                    return;
                };
                super::requests::reannounce_route(&ctx);
            }
        }
    })?;
    let ctx = ctx.this.clone();
    scope.spawn_with_token("call route heal", move |stop| async move {
        loop {
            let peer = tokio::select! {
                biased;
                () = stop.cancelled() => return,
                next = updates.recv() => match next {
                    Ok(peer) => peer,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            let Some(ctx) = ctx.upgrade() else { return };
            let transport = {
                let call = ctx.voice.call.lock();
                if call.as_ref().is_none_or(|call| call.peer != peer) {
                    continue;
                }
                ctx.voice.transport()
            };
            if let (Some(transport), Some(route)) = (
                transport,
                rekindle_transport::operations::dm::route::cached(&node, &peer),
            ) {
                transport.lock().await.refresh_peer_route(&peer, &route);
            }
        }
    })
}
