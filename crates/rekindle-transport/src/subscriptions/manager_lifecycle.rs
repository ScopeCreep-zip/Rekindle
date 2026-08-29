//! Background loop lifecycle: watch renewal, poll (tier 3 fallback),
//! and shutdown.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::info;

use super::events::{self, SubscriptionEvent};
use super::{poll, watches, SubscriptionManager};

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
        self.watches.write().entries.clear();
        info!("subscription manager shut down");
    }
}
