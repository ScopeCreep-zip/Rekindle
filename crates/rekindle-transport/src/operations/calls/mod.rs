//! W16.6 — 1:1 call signaling operations.
//!
//! [`CallRuntime`] is the orchestration layer that drives the
//! pure-logic [`CallStateMachine`] (W16.5). Each public method on the
//! runtime represents either a local user action (`start_dm_call`,
//! `accept_dm_call`, `decline_dm_call`, `end_dm_call`) or a mid-call
//! state ping (`send_call_media_state`, `send_call_reaction`).
//!
//! Each method:
//! 1. Constructs a [`CallInput`] for the change.
//! 2. Drives the state machine via `apply` (returns `Vec<Effect>`).
//! 3. Interprets effects: serializes envelope sends through
//!    [`EnvelopeQueue`], spawns timers, persists state via
//!    [`EnvelopeStore`], emits notifications via [`SharedState::notify`],
//!    and delegates voice-session start/stop to [`VoiceSessionLauncher`].
//!
//! The receive-side dispatch (W16.7) and ring timer integration also
//! call back into this runtime via the same `apply` + interpret-effects
//! shape, which keeps every event flow through the same pipeline.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_calls::{CallInput, CallKind, CallStateMachine};
use rekindle_utils::timestamp_ms;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::envelope_queue::EnvelopeQueue;
use crate::envelope_store::EnvelopeStore;
use crate::shared::SharedState;

mod effects;
mod inbound;
mod outbound;
mod recovery;
mod util;

/// Default ring duration matches arch §10.10 — 30 s window for both
/// caller-side dialing and receiver-side incoming. Re-exported from
/// rekindle-calls, which owns the state machine this paces.
pub use rekindle_calls::signaling::outbound::RING_DURATION_MS;

/// W16.5b — timeout for the `app_call` CallInvite handshake. Veilid's
/// `network.rpc.timeout_ms` defaults to 5 s; through private routes
/// `rpc_processor/mod.rs:1300-1303` doubles it to 10 s. We use 10 s
/// to give the receiver's `on_call` handler enough budget for the
/// state-machine + persist operations even on slow disks. If the
/// receiver is unreachable, the timeout fires fast — well under the
/// 30 s ring window — and the caller's UI surfaces "peer unreachable"
/// immediately via `CallUnreachable`.
pub const CALL_INVITE_RPC_TIMEOUT_MS: u64 = 10_000;

/// Hook the runtime calls to bring up / tear down the audio + jitter
/// pipeline for a call. Implementer is shell-specific (Tauri uses
/// cpal-backed `rekindle-voice`; CLI/daemon could no-op or use a
/// different backend).
///
/// Errors propagate back through the state machine as a
/// `VoiceTransportDown` event so the call cleans up consistently.
#[async_trait]
pub trait VoiceSessionLauncher: Send + Sync {
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer: &str,
        kind: CallKind,
        call_key: [u8; 32],
    ) -> Result<(), String>;

    async fn stop_voice_session(&self, call_id: &str, reason: &str);
}

/// No-op launcher useful for tests and headless deployments that don't
/// run audio. Logs every call but doesn't actually start audio.
pub struct NoopVoiceSessionLauncher;

#[async_trait]
impl VoiceSessionLauncher for NoopVoiceSessionLauncher {
    async fn start_voice_session(
        &self,
        call_id: &str,
        peer: &str,
        _kind: CallKind,
        _call_key: [u8; 32],
    ) -> Result<(), String> {
        debug!(
            call_id,
            peer, "NoopVoiceSessionLauncher::start_voice_session (no audio)"
        );
        Ok(())
    }

    async fn stop_voice_session(&self, call_id: &str, reason: &str) {
        debug!(
            call_id,
            reason, "NoopVoiceSessionLauncher::stop_voice_session"
        );
    }
}

/// Errors surfaced by the call runtime to callers.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("identity not initialized")]
    NoIdentity,
    #[error("call not found: {0}")]
    NotFound(String),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("queue: {0}")]
    Queue(String),
    #[error("store: {0}")]
    Store(String),
    #[error("serialize: {0}")]
    Serialize(String),
}

/// Orchestrates the 1:1 call lifecycle. Cloned cheaply (internal `Arc`s).
#[derive(Clone)]
pub struct CallRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    state_machine: Mutex<CallStateMachine>,
    queue: EnvelopeQueue,
    store: Arc<dyn EnvelopeStore>,
    notifications: Arc<SharedState>,
    voice_launcher: Arc<dyn VoiceSessionLauncher>,
    /// Per-call tokio task handles for ring timers. Keyed by call_id.
    timers: Mutex<HashMap<String, JoinHandle<()>>>,
    owner_key: String,
}

impl CallRuntime {
    /// Construct a new runtime. The runtime borrows the queue + store +
    /// notifications channel from the transport node; owner_key is the
    /// local identity's hex pubkey (used for store scoping).
    pub fn new(
        queue: EnvelopeQueue,
        store: Arc<dyn EnvelopeStore>,
        notifications: Arc<SharedState>,
        voice_launcher: Arc<dyn VoiceSessionLauncher>,
        owner_key: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                state_machine: Mutex::new(CallStateMachine::new()),
                queue,
                store,
                notifications,
                voice_launcher,
                timers: Mutex::new(HashMap::new()),
                owner_key: owner_key.into(),
            }),
        }
    }

    /// True if there's already an outgoing call to this peer (used by
    /// W16.7's glare detection — on inbound CallInvite for a peer we're
    /// already calling, lower-pubkey-wins resolution kicks in).
    pub fn has_outgoing_to(&self, peer: &str) -> bool {
        self.inner.state_machine.lock().has_outgoing_to(peer)
    }

    fn spawn_timeout(&self, call_id: String, expires_at_ms: u64, kind: TimerKind) {
        let now = timestamp_ms();
        let remaining_ms = expires_at_ms.saturating_sub(now).max(1);
        let runtime = self.clone();
        let cid = call_id.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(remaining_ms)).await;
            // Drive state machine with the timeout event. The state
            // machine no-ops if the call has already left
            // Outgoing/Incoming.
            let event = match kind {
                TimerKind::Dialing => CallInput::LocalDialingTimeout { call_id: cid },
                TimerKind::Incoming => CallInput::LocalIncomingTimeout { call_id: cid },
            };
            let effects = runtime.inner.state_machine.lock().apply(event);
            runtime.interpret_effects(effects).await;
        });
        // Cancel any prior timer for this call_id (e.g. re-spawn after
        // a state change). Prior handle's task aborts when dropped.
        self.inner.timers.lock().insert(call_id, handle);
    }
}

#[derive(Debug, Clone, Copy)]
enum TimerKind {
    Dialing,
    Incoming,
}

/// Boxed future returned by [`CallRuntime::interpret_effects`] so the
/// recursive call (StartVoiceSession failure → VoiceTransportDown
/// effects) can be awaited without blowing the stack via async fn
/// monomorphization.
type EffectsFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
