//! Phase 13 — dependency traits for the DM domain.
//!
//! The DM business logic in `rekindle-dm` (sender, receiver, session,
//! ingest) is parameterized over these traits so the crate stays free of
//! `veilid-core`, `AppState`, `tauri::AppHandle`, and `tokio_rusqlite`
//! types at its public surface. The src-tauri adapter
//! (`services/dm_adapter.rs`) implements them against the live Veilid
//! runtime + Signal session manager + Tauri AppHandle.
//!
//! Why split into multiple traits rather than one god-trait:
//! - `DmStore` (existing, in `store.rs`) is the persistence port.
//! - `DmMekCache` is the per-record MEK chain registry — held by the
//!   adapter on `AppState`, accessed via concrete operations (insert,
//!   current, observed_and_lookup, advance) so the trait stays
//!   dyn-safe.
//! - `DmDeps` is the orchestration port: DHT ops + transport ops +
//!   identity ops + event emit. One trait because every DM operation
//!   needs some subset of these and splitting further would force
//!   callers to thread multiple `&dyn _` parameters.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::message::envelope::MessagePayload;
use rekindle_records::lease::LeaseId;

use crate::error::DmError;
use crate::mek::DmMekChain;
use crate::store::DmStore;

/// Per-record MEK chain registry. The DM crate owns the `DmMekChain`
/// type but not its lifetime — the adapter holds the registry on
/// `AppState` so the chain survives across multiple sender/receiver
/// calls within a session.
///
/// All methods are concrete (no generics) so the trait is object-safe.
pub trait DmMekCache: Send + Sync {
    /// Install a fresh chain at conversation create/accept time.
    fn insert(&self, record_key: &str, chain: DmMekChain);

    /// Read the current outbound generation's MEK bytes + generation
    /// number. Errors if no chain is cached for this record.
    fn current(&self, record_key: &str) -> Result<([u8; 32], u64), DmError>;

    /// Forward-lock our writer generation on inbound observation, then
    /// return the MEK bytes for the observed generation. Used by the
    /// receiver to decrypt envelopes at the sender's generation.
    fn observed_and_lookup(&self, record_key: &str, observed_gen: u64)
        -> Result<[u8; 32], DmError>;

    /// Advance the chain to the next generation (called on ratchet
    /// trigger). Returns the new generation number.
    fn advance(&self, record_key: &str) -> Result<u64, DmError>;
}

/// DM-side events the domain logic emits. The src-tauri adapter maps
/// each variant to its `ChatEvent` / `community-event` counterpart and
/// calls `event_emit::emit_live`.
#[derive(Debug, Clone)]
pub enum DmEvent {
    /// A new DM message arrived (either inbound decrypted or
    /// outbound local echo) — UI should render in the conversation.
    MessageReceived {
        record_key: String,
        sender_pseudonym: String,
        body: String,
        timestamp_ms: u64,
    },
    /// An inbound DM invite arrived — UI should surface the request.
    InviteReceived {
        record_key: String,
        sender_pseudonym: String,
        sender_public_key_hex: String,
        is_group: bool,
    },
    /// A peer declined our outbound invite.
    InviteDeclined { record_key: String, reason: String },
    /// A peer left a group DM.
    GroupMemberLeft {
        record_key: String,
        sender_public_key_hex: String,
    },
    /// A complete DM video frame was assembled from incoming fragments
    /// (architecture §W11.4). Adapter emits a `dm-video-frame` event
    /// with the base64-encoded payload for the frontend decoder.
    VideoFrameAssembled {
        sender_public_key_hex: String,
        stream_id: [u8; 16],
        frame_seq: u32,
        keyframe: bool,
        /// Codec wire string — the receiver's decoder follows this tag.
        codec: String,
        timestamp: u32,
        data: Vec<u8>,
    },
}

/// Orchestration port. The DM crate calls these methods to perform
/// every external operation — DHT writes, app-message sends, identity
/// extraction, frontend emits. The adapter handles all the
/// `veilid-core` / `AppState` / `AppHandle` plumbing.
#[async_trait]
pub trait DmDeps: Send + Sync + 'static {
    // --- Identity ---

    /// Owner key for multi-user scoping (the current identity's public
    /// key as a hex string). Errors if no identity is loaded.
    fn owner_key(&self) -> Result<String, DmError>;

    /// Identity Ed25519 secret bytes (32 B). Errors if no identity is
    /// loaded. Caller is responsible for zeroizing after use.
    fn identity_secret(&self) -> Result<[u8; 32], DmError>;

    // --- Subsystems ---

    /// Persistence handle.
    fn store(&self) -> Arc<dyn DmStore>;

    /// Per-record MEK chain registry.
    fn mek_cache(&self) -> Arc<dyn DmMekCache>;

    // --- DHT operations (Veilid is hidden behind these abstractions) ---
    //
    // Records are borrowed from the host's record pool (plan C7.5): every
    // write and watch names a lease. A DM keeps the lease that carries its
    // watch for the session (`dht_hold_session`); a send borrows and
    // releases.

    /// Create a fresh SMPL DHT record with the given member slot pubkeys
    /// (one per subkey, in order), held under its creator lease. Returns
    /// the lease and the record key's canonical string form.
    async fn dht_create_smpl_record(
        &self,
        member_pubkeys: Vec<[u8; 32]>,
    ) -> Result<(LeaseId, String), DmError>;

    /// Borrow an existing DHT record, writable with `writer_keypair` (slot
    /// Ed25519 secret + public bytes) when given.
    async fn dht_acquire_record(
        &self,
        record_key: &str,
        writer_keypair: Option<([u8; 32], [u8; 32])>,
    ) -> Result<LeaseId, DmError>;

    /// End a borrow; the record closes when it was the last.
    async fn dht_release_record(&self, lease: LeaseId);

    /// Write `value` to `subkey` of the leased record, signed by the
    /// supplied slot keypair.
    async fn dht_write_subkey(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer_keypair: ([u8; 32], [u8; 32]),
    ) -> Result<(), DmError>;

    /// Watch the listed subkeys of the leased record. The adapter routes
    /// resulting `ValueChange` events back into the DM dispatch path
    /// (`handle_dm_subkey_change`).
    async fn dht_watch_subkeys(&self, lease: LeaseId, subkeys: Vec<u32>) -> Result<(), DmError>;

    /// Hand the DM's watch lease to the host, which keeps it for the
    /// session (a lease on a record it already holds is released).
    async fn dht_hold_session(&self, record_key: &str, lease: LeaseId);

    /// Release the DM's held lease (the conversation ended: declined or
    /// left). A no-op when none is held.
    async fn dht_release_session(&self, record_key: &str);

    // --- Transport ---

    /// Send a Signal-encrypted `app_call` to a peer and wait for the
    /// reply. Used for DM invites (synchronous accept/decline reply).
    /// The adapter serializes via `serde_json` to match the existing
    /// `message_service::send_to_peer_call` wire format.
    async fn send_app_call(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<MessagePayload, DmError>;

    /// Send a Signal-encrypted `app_message` (fire-and-forget) to a
    /// peer. Used for DM video fragments + future encrypted controls.
    async fn send_encrypted(
        &self,
        peer_pubkey_hex: &str,
        payload: MessagePayload,
    ) -> Result<(), DmError>;

    // --- Frontend emit ---

    /// Push a DM event to the frontend channel. The adapter maps each
    /// `DmEvent` variant to its concrete Tauri `ChatEvent` / community
    /// event payload and calls `event_emit::emit_live`.
    fn emit_event(&self, event: DmEvent);
}
