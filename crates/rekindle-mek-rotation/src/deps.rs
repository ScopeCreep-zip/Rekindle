//! Phase 17 — MekDistributeDeps + ChannelMekCache + MekPersist traits.
//!
//! The cascade rotation orchestrator parameterises over `MekDistributeDeps`
//! so the crate never touches `AppState` / `tauri::AppHandle` /
//! `veilid-core` directly (Invariant 2). The src-tauri `MekAdapter`
//! supplies the live wiring (task #148).

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::id::PseudonymKey;

use crate::error::MekRotationError;
use crate::event::MekRotationEvent;

/// A peer eligible to receive a wrapped MEK envelope. `route_blob` is
/// the importable Veilid route bytes (adapter-supplied); the crate
/// just threads it through `broadcast_to_peer`.
#[derive(Debug, Clone)]
pub struct RotationRecipient {
    pub pseudonym_hex: String,
    pub route_blob: Vec<u8>,
}

/// In-memory MEK cache backing `ChannelMekCache`. The src-tauri
/// adapter implements this against the existing
/// `state.channel_mek_cache: Mutex<HashMap<(String, String), MediaEncryptionKey>>`.
pub trait ChannelMekCache: Send + Sync {
    /// Return the cached MEK for `(community, channel)` at the
    /// matching `generation`. None if the cache holds a different
    /// generation or no entry.
    fn get(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Option<MediaEncryptionKey>;

    /// Replace (or insert) the cached MEK for `(community, channel)`.
    /// The existing generation is overwritten — callers must not
    /// downgrade.
    fn insert(&self, community_id: &str, channel_id: &str, mek: MediaEncryptionKey);

    /// Convenience: return the current cached generation for
    /// `(community, channel)`, or 0 if no entry.
    fn current_generation(&self, community_id: &str, channel_id: &str) -> u64;
}

/// Durable MEK persistence — the keystore-backed store that survives
/// process restarts. Used at rotation time to write the new
/// generation's wrapped bytes; used on cold-start to repopulate the
/// in-memory cache.
#[async_trait]
pub trait MekPersist: Send + Sync {
    /// Store the wrapped MEK bytes for `(community, channel,
    /// generation)`. Returns Ok even if the row already exists.
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
        wrapped_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError>;

    /// Load the wrapped MEK bytes for `(community, channel,
    /// generation)` if previously stored.
    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Result<Option<Vec<u8>>, MekRotationError>;
}

/// Single deps trait for the cascade rotation orchestrator. Composes
/// the cache + persist trait objects (so the adapter can swap impls
/// independently of the orchestrator wiring) plus the I/O methods
/// the rotator + receiver paths need: pseudonym lookup, online-peer
/// enumeration, broadcast, event emit, Lamport clock.
#[async_trait]
pub trait MekDistributeDeps: Send + Sync {
    /// In-memory MEK cache (typically a parking_lot-backed HashMap on
    /// AppState).
    fn cache(&self) -> Arc<dyn ChannelMekCache>;

    /// Durable MEK store (keystore-backed).
    fn persist(&self) -> Arc<dyn MekPersist>;

    /// Local member's pseudonym for `community_id`. None if not a
    /// member or identity is locked.
    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey>;

    /// Snapshot of online recipients in the community. `exclude_pseudonym`
    /// is the departed/triggering peer the rotation should skip.
    fn online_recipients(
        &self,
        community_id: &str,
        exclude_pseudonym: Option<&str>,
    ) -> Vec<RotationRecipient>;

    /// Voice-channel-scoped recipients — voice MEK rotation only
    /// targets peers currently in the voice channel transport.
    fn voice_recipients(
        &self,
        community_id: &str,
        channel_id: &str,
        trigger_pseudonym: &str,
        include_trigger_in_recipients: bool,
    ) -> Vec<RotationRecipient>;

    /// Deliver a wrapped-MEK envelope to a single peer. The adapter
    /// imports the route_blob, builds an `app_call`, and returns the
    /// reply bytes so the caller can inspect ACK variants
    /// (`MekTransferAck`) for delivery confirmation.
    async fn broadcast_to_peer(
        &self,
        community_id: &str,
        peer_pseudonym_hex: &str,
        route_blob: &[u8],
        envelope_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, MekRotationError>;

    /// Emit a UI-facing rotation event.
    fn emit_event(&self, event: MekRotationEvent);

    /// Current per-community Lamport counter (read-only).
    fn current_lamport(&self, community_id: &str) -> u64;

    /// Increment + return the per-community Lamport counter. Used by
    /// the rotator to stamp the wrap envelope.
    fn increment_lamport(&self, community_id: &str) -> u64;

    /// Identity secret bytes (for deriving the rotator's own
    /// pseudonym signing key in distribute.rs).
    fn identity_secret(&self) -> Option<[u8; 32]>;

    /// Apply a received MEK to the cache + bump the matching generation
    /// state. `channel_id = None` (or empty) targets the community-wide
    /// MEK; `Some(ch)` targets the per-channel MEK.
    fn apply_received_mek_to_state(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &MediaEncryptionKey,
    );

    /// Persist a received MEK to the keystore so it survives restart.
    fn persist_received_mek(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        mek: &MediaEncryptionKey,
    );

    /// UI-facing rotation event for an *incoming* MEK transfer (sender
    /// is a remote peer). Distinct from `emit_event` (used for
    /// rotator-initiated lifecycle states like `RotationStarted`).
    fn emit_rotation_received(
        &self,
        community_id: &str,
        channel_id: Option<&str>,
        generation: u64,
    );

    /// Write a governance entry to the merged CRDT state. Used by
    /// `rotate_text_mek_for_departure` to stamp the
    /// `MEKGenerationBump` entry on the rotator side.
    async fn write_governance_entry(
        &self,
        community_id: &str,
        entry: rekindle_types::governance::GovernanceEntry,
    ) -> Result<(), MekRotationError>;

    /// Fan out a `CommunityEnvelope` to the mesh. Used by rotation
    /// orchestrators to broadcast `MEKRotated` after a successful
    /// distribute round.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &rekindle_protocol::dht::community::envelope::CommunityEnvelope,
    ) -> Result<(), MekRotationError>;
}
