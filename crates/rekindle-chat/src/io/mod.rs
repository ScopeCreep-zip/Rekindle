//! PlatformIO — the sole outbound I/O commodity layer for the Rekindle platform.
//!
//! Every service (messaging, friendship, community, identity, presence, voice,
//! and every future feature module) holds `Arc<PlatformIO>` and calls its
//! methods for all network operations. No service directly calls transport
//! methods, constructs gossip envelopes, or accesses the signing key.
//!
//! PlatformIO owns: signing key lifecycle, envelope construction, TypeId framing,
//! postcard serialization, gossip envelope signing, transport dispatch, write
//! verification, and propagation confirmation.
//!
//! The signing key is PlatformIO's internal concern. It starts as None
//! (daemon locked / uninitialized). `set_signing_key()` is called during
//! unlock/resume. `clear_signing_key()` is called during lock/shutdown.
//! The `Arc<RwLock<Option<SigningKeyHandle>>>` is born inside PlatformIO
//! and never leaves — no external code holds or mutates it.

pub mod gossip;
pub mod peer_notify;
pub mod dht;
pub mod route;
pub mod identity;

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use rekindle_types::transport::Transport;

use crate::crypto::SigningKeyHandle;
use crate::ChatError;

/// How thoroughly to verify an outbound operation succeeded.
///
/// Decentralized networks have no authoritative server to confirm writes.
/// This enum lets each operation specify the confidence level it requires.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// Fire and forget. Return Ok as soon as the transport accepts the bytes.
    /// Use for: typing indicators, presence heartbeats, voice packets.
    None,

    /// Wait for the transport to acknowledge delivery. This is the default.
    /// Veilid: app_message returned Ok, set_dht_value returned Ok.
    #[default]
    Accepted,

    /// After transport acknowledges, read back the value and verify it matches
    /// what was written. For DHT writes: get_dht_value after set_dht_value,
    /// compare content. Detects concurrent write conflicts.
    /// Use for: governance writes, member registry, friend inbox writes.
    Verified,

    /// Verified + wait for at least one remote node to confirm they hold the
    /// value. Veilid: inspect_record after set, confirm remote seq >= local seq.
    /// Use for: identity creation, MEK rotation — values that must be
    /// discoverable by other nodes before the operation is considered complete.
    Propagated,
}

/// Result of a DHT write operation with confirmation metadata.
#[derive(Debug)]
pub struct WriteReceipt {
    pub key: String,
    pub subkey: u32,
    pub confirmed: Confirm,
    pub verified: bool,
    pub remote_holders: u32,
    pub elapsed: Duration,
}

/// Result of a gossip broadcast.
#[derive(Debug)]
pub struct BroadcastReceipt {
    pub peers_sent: u32,
    pub peers_failed: u32,
    pub elapsed: Duration,
}

/// Result of a peer-to-peer send.
#[derive(Debug)]
pub struct SendReceipt {
    pub peer_key: String,
    pub confirmed: Confirm,
    pub elapsed: Duration,
}

/// The sole outbound I/O interface for all application logic.
///
/// Constructed once per daemon lifetime with `PlatformIO::new(transport)`.
/// The signing key starts as None and is set/cleared during the daemon
/// lifecycle via `set_signing_key()` / `clear_signing_key()`.
///
/// All services hold `Arc<PlatformIO>`. When the signing key is set,
/// every service's signing operations immediately start working. When
/// cleared, they all immediately return `ChatError::SigningKeyNotLoaded`.
pub struct PlatformIO {
    transport: Arc<dyn Transport>,
    signing_key: Arc<RwLock<Option<SigningKeyHandle>>>,
}

impl PlatformIO {
    /// Construct a new PlatformIO. The signing key starts as None.
    ///
    /// Call `set_signing_key()` during unlock/resume after loading the
    /// key from vault. Call `clear_signing_key()` during lock/shutdown.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            signing_key: Arc::new(RwLock::new(None)),
        }
    }

    // ── Signing key lifecycle ───────────────────────────────────

    /// Set the signing key. Called during unlock/resume after loading
    /// the key from vault. All services sharing this PlatformIO
    /// immediately gain signing capability.
    ///
    /// If a signing key was already set (e.g., from a previous unlock
    /// cycle without an intervening clear), the old handle is dropped
    /// and ZeroizeOnDrop fires on the old key material.
    pub fn set_signing_key(&self, handle: SigningKeyHandle) {
        let mut guard = self.signing_key.write();
        *guard = Some(handle);
        tracing::debug!("signing key loaded on PlatformIO");
    }

    /// Clear the signing key. Called during lock/shutdown.
    /// ZeroizeOnDrop fires on the old SigningKeyHandle, zeroing
    /// the key material in memory.
    ///
    /// After this call, all signing operations return
    /// `ChatError::SigningKeyNotLoaded` until `set_signing_key`
    /// is called again.
    pub fn clear_signing_key(&self) {
        let mut guard = self.signing_key.write();
        *guard = None;
        tracing::debug!("signing key cleared from PlatformIO");
    }

    /// Whether the signing key is currently loaded.
    pub fn is_signing_key_loaded(&self) -> bool {
        self.signing_key.read().is_some()
    }

    // ── Transport diagnostics ─────────────────────────────────────

    /// Whether the transport is attached to the network.
    pub fn is_attached(&self) -> bool {
        self.transport.is_attached()
    }

    /// Peer count from the transport layer.
    pub fn peer_count(&self) -> u32 {
        self.transport.peer_count()
    }

    /// Transport uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.transport.uptime_secs()
    }

    /// Transport attachment state as a human-readable string.
    pub fn attachment_state(&self) -> &str {
        self.transport.attachment_state()
    }

    /// Access the raw transport. Escape hatch for operations not yet
    /// promoted to named PlatformIO methods. Every use of this method
    /// is a candidate for promotion — track usages.
    pub fn transport(&self) -> &Arc<dyn Transport> {
        &self.transport
    }

    // ── Internal signing key access ─────────────────────────────

    /// Require the signing key seed bytes. Returns
    /// `ChatError::SigningKeyNotLoaded` if the daemon is locked.
    pub(crate) fn require_signing_key(&self) -> Result<[u8; 32], ChatError> {
        let guard = self.signing_key.read();
        let handle = guard.as_ref().ok_or(ChatError::SigningKeyNotLoaded)?;
        Ok(*handle.as_bytes())
    }

    /// Execute a closure with a reference to the signing key handle.
    /// Returns `ChatError::SigningKeyNotLoaded` if the daemon is locked.
    ///
    /// Use when multiple derivations are needed from the same key to
    /// avoid copying the seed for each derivation.
    pub(crate) fn with_signing_key<F, R>(&self, f: F) -> Result<R, ChatError>
    where
        F: FnOnce(&SigningKeyHandle) -> Result<R, ChatError>,
    {
        let guard = self.signing_key.read();
        let handle = guard.as_ref().ok_or(ChatError::SigningKeyNotLoaded)?;
        f(handle)
    }
}

impl std::fmt::Debug for PlatformIO {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformIO")
            .field("transport_attached", &self.transport.is_attached())
            .field("signing_key_loaded", &self.signing_key.read().is_some())
            .finish()
    }
}
