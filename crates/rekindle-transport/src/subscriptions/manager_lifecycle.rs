//! Background loop lifecycle: maintenance (typing expiry + dedup
//! eviction) and shutdown.

use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::ScopeClosed;
use tracing::{info, warn};

use super::events::{self, SubscriptionEvent};
use super::SubscriptionManager;

/// How often the maintenance loop sweeps. Matches the typing expiry
/// deadline in `rekindle_events::state`, the shorter of the two it
/// services.
const TYPING_SWEEP_SECS: u64 = 5;

/// How long the background loops get to stop at shutdown.
const STOP_DEADLINE: Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

impl SubscriptionManager {
    /// Sweep expired typing indicators and dedup entries.
    ///
    /// `TypingEvent::Stopped` documents itself as "triggered by: expiry
    /// timer" — there was no timer. `collect_expired_channel_typers` and
    /// `EventDedup::evict_expired` were written and never called, so a
    /// member who stopped typing stayed "typing…" until they typed again,
    /// and the dedup cache only ever shed entries by hitting its capacity
    /// bound. DM typing expires in the Host (`operations::dm::typing`).
    ///
    /// Ticks at the typing expiry interval because that is the shorter
    /// of the two deadlines; the dedup TTL is far longer and sweeping it
    /// more often than necessary costs one pass over an ordered deque
    /// that stops at the first live entry.
    ///
    /// # Errors
    /// [`ScopeClosed`] when the manager's scope was already shut down.
    pub fn start_maintenance_loop(&self) -> Result<(), ScopeClosed> {
        let state = Arc::clone(&self.state);
        let dedup = Arc::clone(&self.dedup);
        let event_tx = self.event_tx.clone();

        self.scope.spawn_with_token("subscription maintenance", |stop| async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(TYPING_SWEEP_SECS));
            interval.tick().await; // skip the immediate first tick
            loop {
                // Biased, stop first: a stop beside a tick sweeps no more.
                tokio::select! {
                    biased;
                    () = stop.cancelled() => {
                        info!("maintenance loop shutting down");
                        break;
                    }
                    _ = interval.tick() => {
                        // Guards dropped before sending: `process_event`
                        // is not reachable from here, but the broadcast
                        // send can block and these are parking_lot
                        // guards.
                        let channel_expired =
                            state.write().typing.collect_expired_channel_typers();
                        dedup.write().evict_expired();

                        for (community, channel, who) in channel_expired {
                            let _ = event_tx.send(SubscriptionEvent::Typing(
                                events::TypingEvent::Stopped {
                                    context: events::TypingContext::Channel { community, channel },
                                    who,
                                },
                            ));
                        }
                    }
                }
            }
        })?;

        info!("maintenance loop started (typing expiry + dedup eviction)");
        Ok(())
    }

    /// Shut down: stop all background loops, clear all state.
    pub async fn shutdown(&self) {
        if let Err(stuck) = self.scope.shutdown(STOP_DEADLINE).await {
            warn!(%stuck, "subscription loops did not stop in time");
        }
        info!("subscription manager shut down");
    }
}
