//! PlatformIO — the sole outbound I/O commodity layer for the Rekindle platform.
//!
//! Every service (messaging, friendship, community, identity, presence, voice,
//! and every future feature module) holds `Arc<PlatformIO>` and calls its
//! methods for all network operations. No service directly calls transport
//! methods, constructs gossip envelopes, or accesses raw key material.
//!
//! PlatformIO owns: identity lifecycle, envelope construction, TypeId framing,
//! postcard serialization, gossip envelope signing, transport dispatch, write
//! verification, and propagation confirmation.
//!
//! The `SelfIdentity` is PlatformIO's internal concern. It starts as None
//! (daemon locked / uninitialized). `set_identity()` is called during
//! unlock/resume. `clear_identity()` is called during lock/shutdown.
//! The `Arc<RwLock<Option<SelfIdentity>>>` is born inside PlatformIO
//! and never leaves — no external code holds or mutates it.

pub mod gossip;
pub mod peer_notify;
pub mod dht;
pub mod route;
pub mod identity;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use rekindle_types::transport::Transport;

use rekindle_identity::self_id::SelfIdentity;
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
/// The identity starts as None and is set/cleared during the daemon
/// lifecycle via `set_identity()` / `clear_identity()`.
///
/// All services hold `Arc<PlatformIO>`. When the signing key is set,
/// every service's signing operations immediately start working. When
/// cleared, they all immediately return `ChatError::IdentityNotLoaded`.
pub struct PlatformIO {
    transport: Arc<dyn Transport>,
    self_identity: Arc<RwLock<Option<SelfIdentity>>>,
    /// Keys of DHT records opened this session. Checked by `open_record`
    /// to skip redundant transport calls. Cleared on `clear_identity`.
    pub(crate) open_keys: Mutex<HashSet<String>>,
}

impl PlatformIO {
    /// Construct a new PlatformIO. The signing key starts as None.
    ///
    /// Call `set_identity()` during unlock/resume after constructing
    /// SelfIdentity. Call `clear_identity()` during lock/shutdown.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self {
            transport,
            self_identity: Arc::new(RwLock::new(None)),
            open_keys: Mutex::new(HashSet::new()),
        }
    }

    // ── Identity lifecycle ──────────────────────────────────────

    /// Set the identity. Called during unlock/resume after loading
    /// the seed from vault and constructing SelfIdentity.
    /// All services sharing this PlatformIO immediately gain
    /// signing and derivation capability.
    ///
    /// If an identity was already set (e.g., from a previous unlock
    /// cycle without an intervening clear), the old SelfIdentity is
    /// dropped and OriginSeed's ZeroizeOnDrop fires.
    pub fn set_identity(&self, identity: SelfIdentity) {
        let mut guard = self.self_identity.write();
        *guard = Some(identity);
        tracing::debug!("identity loaded on PlatformIO");
    }

    /// Clear the identity. Called during lock/shutdown.
    /// OriginSeed's ZeroizeOnDrop fires, zeroing the seed in memory.
    ///
    /// After this call, all identity operations return
    /// `ChatError::IdentityNotLoaded` until `set_identity`
    /// is called again.
    pub fn clear_identity(&self) {
        let mut guard = self.self_identity.write();
        *guard = None;
        self.open_keys.lock().clear();
        tracing::debug!("identity cleared from PlatformIO — open record cache flushed");
    }

    /// Whether the identity is currently loaded.
    pub fn is_identity_loaded(&self) -> bool {
        self.self_identity.read().is_some()
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

    // ── Internal identity access ────────────────────────────────

    /// Execute a closure with a reference to the SelfIdentity.
    /// Returns `ChatError::IdentityNotLoaded` if the daemon is locked.
    ///
    /// This is the ONLY way to access the identity from service code.
    /// The closure pattern ensures the RwLock guard is dropped before
    /// any async work.
    pub(crate) fn with_identity<F, R>(&self, f: F) -> Result<R, ChatError>
    where
        F: FnOnce(&SelfIdentity) -> Result<R, ChatError>,
    {
        let guard = self.self_identity.read();
        let si = guard.as_ref().ok_or(ChatError::IdentityNotLoaded)?;
        f(si)
    }

    /// Get the signing keypair from the identity. Convenience wrapper
    /// that reconstructs the Ed25519 keypair for PQXDH operations.
    pub(crate) fn signing_keypair(&self) -> Result<rekindle_identity::SigningKeypair, ChatError> {
        self.with_identity(|si| {
            si.signing_keypair().map_err(|e| ChatError::Internal(format!("signing keypair: {e}")))
        })
    }

    /// Get the X25519 identity seed for PQXDH initiate/respond.
    pub(crate) fn x25519_identity_seed(&self) -> Result<zeroize::Zeroizing<[u8; 32]>, ChatError> {
        self.with_identity(|si| Ok(si.x25519_identity_seed()))
    }

    /// Get the identity root as IdentityRoot.
    pub(crate) fn identity_root(&self) -> Result<rekindle_identity::IdentityRoot, ChatError> {
        self.with_identity(|si| Ok(*si.root()))
    }
}

impl std::fmt::Debug for PlatformIO {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformIO")
            .field("transport_attached", &self.transport.is_attached())
            .field("identity_loaded", &self.self_identity.read().is_some())
            .field("open_records", &self.open_keys.lock().len())
            .finish()
    }
}
