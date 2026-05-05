//! Plan §Failure 5 — pending-accept oneshot table for direct calls.
//!
//! When a `CallOffer` arrives via `app_call`, the inbound handler
//! parks itself on a oneshot channel waiting for the user to accept or
//! decline. The frontend's `accept_dm_call` / `decline_dm_call` Tauri
//! commands look up the matching sender here and resolve the future
//! so the original `app_call` reply can ship `CallAccept` / `CallDecline`
//! back to the caller in band.
//!
//! Process-global because the Tauri command and the inbound handler
//! run in different async tasks; using `AppState` would require
//! threading `SharedState` into a deeply-nested Veilid dispatch loop.

use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::Mutex;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum IncomingDecision {
    Accept,
    Decline(String),
}

static PENDING: OnceLock<Mutex<HashMap<String, oneshot::Sender<IncomingDecision>>>> =
    OnceLock::new();

fn map() -> &'static Mutex<HashMap<String, oneshot::Sender<IncomingDecision>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn insert_pending_response(call_id: &str, tx: oneshot::Sender<IncomingDecision>) {
    map().lock().insert(call_id.to_string(), tx);
}

pub fn take_pending_response(call_id: &str) -> Option<oneshot::Sender<IncomingDecision>> {
    map().lock().remove(call_id)
}
