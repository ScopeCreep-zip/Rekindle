//! Typing the Host receives (plan E2.3): a start is published once and
//! lapses in the backend.
//!
//! The state is `rekindle_events::typing::TypingExpiry` on the Host's
//! [`FriendStores`](crate::operations::friend::FriendStores); a start that
//! is news arms one timer, which publishes the stop when the start was
//! not refreshed in time, so no frontend keeps a timer. Keyed by
//! [`TypingContext`], so E2.6's group typing and E3.4's channel typing
//! (with its own per-start TTL) arrive here too.

use std::time::Duration;

use rekindle_events::typing::Lapse;
use rekindle_types::subscription_events::{SubscriptionEvent, TypingContext, TypingEvent};

use crate::operations::friend::FriendHost;

/// `who` is typing in `context`; the start lapses `ttl_ms` from now unless
/// it is refreshed. Only a new start is published.
pub fn started(host: &dyn FriendHost, context: TypingContext, who: &str, ttl_ms: u64) {
    let now = host.clock().now_ms();
    let news = host
        .stores()
        .typing_in
        .lock()
        .start(context.clone(), who.to_string(), now, ttl_ms);
    if !news {
        return;
    }
    host.publish(SubscriptionEvent::Typing(TypingEvent::Started {
        context: context.clone(),
        who: who.to_string(),
    }));
    watch_lapse(host, context, who.to_string());
}

/// `who` stopped typing in `context` (their stop, or their message
/// arrived). The stop is published when they were typing.
pub fn stopped(host: &dyn FriendHost, context: &TypingContext, who: &str) {
    if host.stores().typing_in.lock().stop(context, who) {
        host.publish(SubscriptionEvent::Typing(TypingEvent::Stopped {
            context: context.clone(),
            who: who.to_string(),
        }));
    }
}

/// Publish the stop once `who`'s start in `context` lapses; a refresh
/// moves the deadline, and a stop or a lock ends the watch.
fn watch_lapse(host: &dyn FriendHost, context: TypingContext, who: String) {
    let owned = host.owned();
    host.scope().spawn_or_drop("typing expiry", async move {
        loop {
            let now = owned.clock().now_ms();
            let lapse = owned.stores().typing_in.lock().lapse(&context, &who, now);
            match lapse {
                Lapse::Pending(deadline) => {
                    tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now))).await;
                }
                Lapse::Ended => {
                    owned.publish(SubscriptionEvent::Typing(TypingEvent::Stopped {
                        context,
                        who,
                    }));
                    return;
                }
                Lapse::Gone => return,
            }
        }
    });
}
