//! Phase 17 — MekDistributeDeps + ChannelMekCache + MekPersist traits.
//!
//! The cascade rotation orchestrator parameterises over `MekDistributeDeps`
//! so the crate never touches `AppState` / `tauri::AppHandle` /
//! `veilid-core` directly (Invariant 2). The src-tauri `MekAdapter`
//! supplies the live wiring (task #148).

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::{ChannelId, PseudonymKey};

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

/// A host's in-memory community and channel keys, addressed by
/// [`KeyScope`]. The desktop implements it over its `mek_cache` (community
/// scope) and `channel_mek_cache` (channel scopes); the daemon over
/// `rekindle_transport::crypto::mek::MekCache`.
pub trait ChannelMekCache: Send + Sync {
    /// The scope's current key.
    fn current(&self, community_id: &str, scope: KeyScope) -> Option<MediaEncryptionKey>;

    /// The scope's key at exactly `generation`, if held in memory. A cache
    /// that keeps only the current key answers from it; one that retains
    /// older generations overrides this.
    fn get(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
    ) -> Option<MediaEncryptionKey> {
        self.current(community_id, scope)
            .filter(|mek| mek.generation() == generation)
    }

    /// Install a key under the convergence rule every host shares: an
    /// older generation is refused (rollback), a newer one replaces, and
    /// an equal generation is kept by the lowest election rank
    /// (`convergence::incoming_wins_same_generation`). Returns whether the
    /// key was installed.
    fn insert(&self, community_id: &str, scope: KeyScope, mek: MediaEncryptionKey) -> bool;

    /// How long the scope's current key has been current, or `None` when
    /// no key is held.
    fn current_age(&self, community_id: &str, scope: KeyScope) -> Option<std::time::Duration>;

    /// The scope's current generation, or 0 when no key is held.
    fn current_generation(&self, community_id: &str, scope: KeyScope) -> u64 {
        self.current(community_id, scope)
            .map_or(0, |mek| mek.generation())
    }
}

/// Durable MEK persistence — the keystore-backed store that survives
/// process restarts.
#[async_trait]
pub trait MekPersist: Send + Sync {
    /// Store the key bytes for `(community, scope, generation)`. Returns
    /// Ok even if the entry already exists.
    async fn store_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
        generation: u64,
        key_bytes: Vec<u8>,
    ) -> Result<(), MekRotationError>;

    /// Load the key bytes for `(community, scope, generation)` if stored.
    async fn load_mek_for_generation(
        &self,
        community_id: &str,
        scope: KeyScope,
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
    /// The scope of the session this rotation belongs to. The cascade wait
    /// and the per-recipient sends stop once it is closed (plan C4.L1).
    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope>;

    /// In-memory MEK cache (typically a parking_lot-backed HashMap on
    /// AppState).
    fn cache(&self) -> Arc<dyn ChannelMekCache>;

    /// Durable MEK store (keystore-backed).
    fn persist(&self) -> Arc<dyn MekPersist>;

    /// Local member's pseudonym for `community_id`. None if not a
    /// member or identity is locked.
    fn my_pseudonym(&self, community_id: &str) -> Option<PseudonymKey>;

    /// Whether `member` may rotate the community key under the merged
    /// governance (`Permissions::may_rotate_mek`, plan D20) — the same
    /// rule readers apply to the resulting `MEKGenerationBump`.
    fn may_rotate(&self, community_id: &str, member: &PseudonymKey) -> bool;

    /// Snapshot of online recipients in the community. `exclude_pseudonym`
    /// is the departed/triggering peer the rotation should skip.
    fn online_recipients(
        &self,
        community_id: &str,
        exclude_pseudonym: Option<&str>,
    ) -> Vec<RotationRecipient>;

    /// Voice-channel-scoped recipients — voice MEK rotation only
    /// targets peers currently in the voice channel transport. Async
    /// because the roster lives behind the transport's async lock and
    /// rotation always runs on the runtime (a blocking read here
    /// panics tokio workers).
    async fn voice_recipients(
        &self,
        community_id: &str,
        channel: ChannelId,
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

    /// Next governance-clock value, for the `MEKGenerationBump` entry.
    fn next_governance_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError>;

    /// Identity secret bytes (for deriving the rotator's own
    /// pseudonym signing key in distribute.rs).
    fn identity_secret(&self) -> Option<[u8; 32]>;

    /// Apply a received MEK to the scope's cache + bump the matching
    /// generation state. Returns whether the key was installed; a key the
    /// convergence rule refuses must not be persisted either, or it would
    /// overwrite the winning key's stored copy.
    fn apply_received_mek_to_state(
        &self,
        community_id: &str,
        scope: KeyScope,
        mek: &MediaEncryptionKey,
    ) -> bool;

    /// Persist a received MEK to the keystore so it survives restart.
    fn persist_received_mek(&self, community_id: &str, scope: KeyScope, mek: &MediaEncryptionKey);

    /// UI-facing rotation event for an *incoming* MEK transfer (sender
    /// is a remote peer). Distinct from `emit_event` (used for
    /// rotator-initiated lifecycle states like `RotationStarted`).
    fn emit_rotation_received(&self, community_id: &str, scope: KeyScope, generation: u64);

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
        envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), MekRotationError>;
}
