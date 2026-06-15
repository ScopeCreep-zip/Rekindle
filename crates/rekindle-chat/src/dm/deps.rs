//! DM orchestration port — the sole interface between dm/ business
//! logic and the runtime.
//!
//! Every dm/ function takes `&dyn DmDeps`. No dm/ file imports
//! PlatformIO, VaultStore, SessionMeta, EventPipeline, SessionCache,
//! or MekCache directly.
//!
//! The impl in `service/mod.rs` bridges these trait methods to the
//! concrete runtime types held by ChatService (`Arc<PlatformIO>`,
//! `Arc<VaultStore>`, `Arc<SessionCache>`, `Arc<EventPipeline>`).
//!
//! Object-safe: no generics, no associated types, no Self by value.

use std::time::Duration;

use async_trait::async_trait;
use rekindle_types::dm_store::DmStore;

use crate::dm::error::DmError;
use crate::dm::mek_chain::DmMekChain;

// ── DM domain events ────────────────────────────────────────────────

/// Events the DM domain logic emits. The `DmDeps` impl maps each variant
/// to `SubscriptionEvent` and pushes through `EventPipeline`.
///
/// This enum is dm/'s output vocabulary. It never carries
/// `SubscriptionEvent` directly — that mapping is the impl's job.
#[derive(Debug, Clone)]
pub enum DmEvent {
    /// A DM message was sent or received.
    MessageReceived {
        record_key: String,
        /// Ed25519 public key hex — for session lookup and unread keying.
        peer_key: String,
        sender_pseudonym: String,
        body: String,
        timestamp_ms: u64,
        is_self: bool,
    },

    /// An inbound DM invite arrived.
    InviteReceived {
        record_key: String,
        sender_pseudonym: String,
        sender_public_key_hex: String,
        is_group: bool,
    },

    /// A peer declined our outbound invite.
    InviteDeclined {
        record_key: String,
        reason: String,
    },

    /// A peer left a group DM.
    GroupMemberLeft {
        record_key: String,
        sender_public_key_hex: String,
    },
}

// ── Group MEK cache (separate trait, single-responsibility) ─────────

/// Per-record MEK chain registry. Group DMs only.
/// 1:1 DMs use Triple Ratchet via `ratchet_encrypt`/`ratchet_decrypt`.
///
/// Separated from `DmDeps` so the cache can be tested/mocked
/// independently. `DmDeps` returns `&dyn DmMekCache`.
pub trait DmMekCache: Send + Sync {
    /// Install a fresh chain at conversation create/accept time.
    fn insert(&self, record_key: &str, chain: DmMekChain);

    /// Current outbound generation's MEK bytes + generation number.
    fn current(&self, record_key: &str) -> Result<([u8; 32], u64), DmError>;

    /// Forward-lock our writer generation on inbound observation,
    /// return the MEK bytes for the observed generation.
    fn observed_and_lookup(
        &self,
        record_key: &str,
        observed_gen: u64,
    ) -> Result<[u8; 32], DmError>;

    /// Advance the chain to the next generation. Returns new gen.
    fn advance(&self, record_key: &str) -> Result<u64, DmError>;
}

// ── Orchestration trait ─────────────────────────────────────────────

/// Every external operation the DM domain logic needs.
///
/// Implemented in `service/mod.rs` on a struct holding the same
/// `Arc` references that `ChatService` holds.
#[async_trait]
pub trait DmDeps: Send + Sync {
    // ── Identity ──────────────────────────────────────────

    /// Our Ed25519 public key as hex.
    fn identity_public_key_hex(&self) -> Result<String, DmError>;

    /// Our Ed25519 public key as raw 32 bytes.
    fn identity_public_key_bytes(&self) -> Result<[u8; 32], DmError>;

    /// Our X25519 DH seed derived from the identity seed.
    fn x25519_identity_seed(&self) -> Result<[u8; 32], DmError>;

    // ── Persistence ───────────────────────────────────────

    /// Access the DM persistence port.
    fn store(&self) -> &dyn DmStore;

    // ── Group MEK cache ───────────────────────────────────

    /// Access the per-record MEK chain registry.
    fn mek_cache(&self) -> &dyn DmMekCache;

    // ── 1:1 Triple Ratchet (opaque) ───────────────────────
    //
    // The dm/ module never sees SessionCache, TripleRatchetSession,
    // or rekindle_ratchet types. If the ratchet implementation changes
    // (e.g., from Triple Ratchet to MLS), only the impl changes.

    /// Encrypt plaintext for a 1:1 peer. The impl loads/creates the
    /// Triple Ratchet session, encrypts, persists ratchet state, handles
    /// skipped keys internally.
    ///
    /// Returns `(encrypted_header, ciphertext)` — both are needed by
    /// the receiver's `triple::decrypt`.
    async fn ratchet_encrypt(
        &self,
        peer_key: &str,
        plaintext: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), DmError>;

    /// Decrypt ciphertext from a 1:1 peer. The impl loads the Triple
    /// Ratchet session, decrypts (with skipped key recovery via
    /// VaultSkippedCallback), persists ratchet state.
    async fn ratchet_decrypt(
        &self,
        peer_key: &str,
        encrypted_header: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, DmError>;

    // ── DHT operations ────────────────────────────────────

    /// Create a SMPL record with the given member slot pubkeys.
    async fn dht_create_smpl_record(
        &self,
        member_pubkeys: Vec<[u8; 32]>,
    ) -> Result<String, DmError>;

    /// Open an existing DHT record for reading.
    async fn dht_open_record(&self, record_key: &str) -> Result<(), DmError>;

    /// Write value to a subkey, signed by the slot keypair.
    async fn dht_write_subkey(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer_keypair: ([u8; 32], [u8; 32]),
    ) -> Result<(), DmError>;

    /// Read a subkey value. Returns None if empty.
    async fn dht_read_subkey(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, DmError>;

    /// Watch subkeys for changes.
    async fn dht_watch_subkeys(
        &self,
        record_key: &str,
        subkeys: Vec<u32>,
    ) -> Result<(), DmError>;

    // ── Peer key discovery ─────────────────────────────────

    /// Read a peer's X25519 DH public key from their profile DHT record.
    /// The X25519 pub is published at `PROFILE_SUBKEY_X25519_PUB`.
    /// Required for group DM MEK derivation (ECDH wrapping).
    /// 1:1 DMs don't need this — the Triple Ratchet session handles keys.
    async fn read_peer_x25519_pub(
        &self,
        peer_profile_key: &str,
    ) -> Result<[u8; 32], DmError>;

    // ── Transport ─────────────────────────────────────────

    /// Synchronous RPC to a peer (fast path for DM invites).
    /// The impl wraps `Transport::call_peer` in `tokio::time::timeout`.
    async fn send_app_call(
        &self,
        peer_pubkey_hex: &str,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, DmError>;

    /// Write a DM invite to the peer's DHT inbox (durable path).
    /// The impl reads the peer's profile for inbox key + keypair,
    /// uses `blake3_hash_mod` for subkey, open + read-append-write +
    /// `Confirm::Accepted`.
    async fn write_dm_invite_to_inbox(
        &self,
        peer_pubkey_hex: &str,
        invite_payload: &[u8],
    ) -> Result<(), DmError>;

    // ── Event emission ────────────────────────────────────

    /// Push a DM event to the frontend.
    fn emit_event(&self, event: DmEvent);
}
