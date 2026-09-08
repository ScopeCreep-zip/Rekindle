//! Background loop lifecycle: watch renewal, poll (tier 3 fallback),
//! maintenance (typing expiry + dedup eviction), and shutdown.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;

use super::events::{self, SubscriptionEvent};
use super::{poll, watches, SubscriptionManager};

/// How often the maintenance loop sweeps. Matches the typing expiry
/// deadline in `rekindle_events::state`, the shorter of the two it
/// services.
const TYPING_SWEEP_SECS: u64 = 5;

impl SubscriptionManager {
    /// Start the background watch renewal loop.
    ///
    /// Renews DHT watches every 60 seconds. Must be called after construction.
    pub fn start_renewal_loop(&mut self) {
        let (tx, rx) = mpsc::channel(1);
        let handle = tokio::spawn(watches::run_renewal_loop(
            Arc::clone(&self.node),
            Arc::clone(&self.watches),
            self.event_tx.clone(),
            rx,
        ));
        self.renewal_handle = Some(handle);
        self.renewal_shutdown_tx = Some(tx);
        info!("subscription watch renewal loop started");
    }

    /// Start the background poll loop (tier 3 fallback).
    ///
    /// Sweeps all watched DHT records every `interval_secs` with force_refresh.
    /// When changes are found, emits `ValueChanged` events through the broadcast
    /// channel. The daemon-internal consumer acts on these to trigger `process_inbox`
    /// and friend inbox scans — completing the tier 3 guarantee for daemon actions.
    pub fn start_poll_loop(&mut self, interval_secs: u64) {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let (change_tx, mut change_rx) = mpsc::channel::<(String, Vec<u32>)>(64);

        // Spawn the poll loop (reads DHT with force_refresh, signals changes)
        let handle = tokio::spawn(poll::run_poll_loop(
            Arc::clone(&self.node),
            Arc::clone(&self.watches),
            interval_secs,
            change_tx,
            shutdown_rx,
        ));

        // Spawn the change consumer (routes poll signals into the event pipeline)
        let event_tx = self.event_tx.clone();
        let dedup = Arc::clone(&self.dedup);
        tokio::spawn(async move {
            while let Some((record_key, changed_subkeys)) = change_rx.recv().await {
                let event = SubscriptionEvent::Network(events::NetworkEvent::ValueChanged {
                    record_key,
                    changed_subkeys,
                });
                // Dedup gate — suppresses if this change was already seen via watch
                if dedup.write().check(&event) {
                    let _ = event_tx.send(event);
                }
            }
        });

        self.poll_handle = Some(handle);
        self.poll_shutdown_tx = Some(shutdown_tx);
        info!(interval_secs, "poll loop started (tier 3 fallback)");
    }

    /// Sweep expired typing indicators and dedup entries.
    ///
    /// `TypingEvent::Stopped` documents itself as "triggered by: expiry
    /// timer" — there was no timer. `collect_expired_channel_typers`,
    /// `collect_expired_dm_typers` and `EventDedup::evict_expired` were
    /// all written and never called, so a peer who stopped typing stayed
    /// "typing…" until they typed again, and the dedup cache only ever
    /// shed entries by hitting its capacity bound.
    ///
    /// Ticks at the typing expiry interval because that is the shorter
    /// of the two deadlines; the dedup TTL is far longer and sweeping it
    /// more often than necessary costs one pass over an ordered deque
    /// that stops at the first live entry.
    pub fn start_maintenance_loop(&mut self) {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        let state = Arc::clone(&self.state);
        let dedup = Arc::clone(&self.dedup);
        let event_tx = self.event_tx.clone();

        let handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(TYPING_SWEEP_SECS));
            interval.tick().await; // skip the immediate first tick
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Guards dropped before sending: `process_event`
                        // is not reachable from here, but the broadcast
                        // send can block and these are parking_lot
                        // guards.
                        let (channel_expired, dm_expired) = {
                            let mut guard = state.write();
                            (
                                guard.typing.collect_expired_channel_typers(),
                                guard.typing.collect_expired_dm_typers(),
                            )
                        };
                        dedup.write().evict_expired();

                        for (community, channel, who) in channel_expired {
                            let _ = event_tx.send(SubscriptionEvent::Typing(
                                events::TypingEvent::Stopped {
                                    context: events::TypingContext::Channel { community, channel },
                                    who,
                                },
                            ));
                        }
                        for peer_key in dm_expired {
                            let _ = event_tx.send(SubscriptionEvent::Typing(
                                events::TypingEvent::Stopped {
                                    context: events::TypingContext::Dm { peer_key: peer_key.clone() },
                                    who: peer_key,
                                },
                            ));
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("maintenance loop shutting down");
                        break;
                    }
                }
            }
        });

        self.maintenance_handle = Some(handle);
        self.maintenance_shutdown_tx = Some(shutdown_tx);
        info!("maintenance loop started (typing expiry + dedup eviction)");
    }

    /// Shut down: stop all background loops, clear all state.
    pub async fn shutdown(&mut self) {
        if let Some(tx) = self.renewal_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.renewal_handle.take() {
            let _ = h.await;
        }
        if let Some(tx) = self.poll_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.poll_handle.take() {
            let _ = h.await;
        }
        if let Some(tx) = self.maintenance_shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        if let Some(h) = self.maintenance_handle.take() {
            let _ = h.await;
        }
        self.watches.write().entries.clear();
        info!("subscription manager shut down");
    }
}
