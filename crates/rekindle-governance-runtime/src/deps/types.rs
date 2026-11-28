//! DTOs exchanged across the `GovernanceRuntimeDeps` boundary.
//!
//! Split from the trait itself to keep both files under the 600-line
//! ceiling. These are the Schwarzschild boundary's vocabulary: every
//! Veilid value crosses as an opaque `String` / `Vec<u8>`, and live
//! host structures cross as snapshots, so the crate never needs the
//! host's state types.

use rekindle_governance::state::GovernanceState;

/// User-status flavor crossing the `Deps` boundary.
///
/// An alias, not a declaration. This was a third copy of the same five
/// variants and the same Invisible→"offline" fold, justified as avoiding
/// a dependency on *src-tauri's* `UserStatus` — but the crate it should
/// have reached for is Tier 1, which it already depends on. Three
/// definitions of one wire vocabulary is three chances for the privacy
/// fold to drift apart.
pub use rekindle_types::presence::SessionStatus as UserStatusKind;

/// Snapshot of the fields of `CommunityState` that lifecycle ops read.
///
/// Batching into a single DTO mirrors how the src-tauri code already
/// accesses state — one locked-read, then multi-field destructure. The
/// adapter clones the parking_lot RwLockReadGuard contents into this
/// DTO so the lock is dropped before any `.await`.
#[derive(Debug, Clone)]
pub struct CommunityMembership {
    pub anchor: Option<String>,
    pub member_registry_key: Option<String>,
    pub my_pseudonym_hex: Option<String>,
    pub my_subkey_index: Option<u32>,
    pub my_segment_index: Option<u32>,
    pub slot_keypair: Option<String>,
    pub slot_seed_hex: Option<String>,
    pub channel_ids: Vec<String>,
    pub mek_generation: u64,
}

/// Online member entry extracted from the gossip overlay's
/// `online_members` map. Adapter snapshots into Vec form so the crate
/// doesn't need access to the live `GossipOverlay` struct.
#[derive(Debug, Clone)]
pub struct OnlineMemberSnapshot {
    pub pseudonym_hex: String,
    pub status: String,
    pub route_blob: Vec<u8>,
    pub last_seen: u64,
}

/// MEK material exchanged across the crate boundary.
///
/// The wire bytes form lets the crate decode/re-encrypt without
/// importing `rekindle_crypto::group::media_key::MediaEncryptionKey` at
/// every callsite (it's used in many places but the trait stays opaque).
#[derive(Debug, Clone)]
pub struct MekSnapshot {
    pub generation: u64,
    /// 32-byte raw key material (the result of `MediaEncryptionKey::as_bytes`).
    pub key_bytes: [u8; 32],
}

/// Outcome of creating a new SMPL DHT record. Owner keypair string is
/// `None` if the underlying record was created with `o_cnt: 0` (the
/// universal community SMPL schema — Schwarzschild principle, §3).
#[derive(Clone)]
pub struct DhtRecordInfo {
    pub record_key: String,
    pub owner_keypair: Option<zeroize::Zeroizing<String>>,
    /// The creator's lease on the new record, held writable: hand it to the
    /// host with `community_records_ready`, or release it.
    pub lease: rekindle_records::lease::LeaseId,
}

impl std::fmt::Debug for DhtRecordInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DhtRecordInfo")
            .field("record_key", &self.record_key)
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

/// Snapshot of the per-community DHT-records info the
/// per-community open (`open_and_track_one_community`) needs. The adapter
/// builds this from a single `state.communities.read()` so the lock
/// is dropped before any `.await`.
#[derive(Debug, Clone)]
pub struct CommunityDhtOpenSetup {
    pub id: String,
    pub anchor: String,
    pub registry_key: Option<String>,
    /// Preferred registry writer keypair string (registry_owner if
    /// present, falling back to the slot keypair). `None` opens the
    /// record read-only.
    pub registry_writer: Option<String>,
}

/// Slot row discovered from a presence/registry scan during join.
#[derive(Debug, Clone)]
pub struct DiscoveredMember {
    pub segment_index: u32,
    pub slot_index: u32,
    pub presence: rekindle_types::presence::MemberPresence,
}

/// Fields needed to insert a freshly-created community (origin flow)
/// into the src-tauri `AppState.communities` map.
///
/// Constructed inside `origin::create_community` once all genesis writes
/// succeed. The adapter copies the contents into a fresh `CommunityState`.
#[derive(Clone)]
pub struct CommunityInsert {
    /// The anchor `A`'s record key.
    pub id: String,
    pub name: String,
    pub channel_id_hex: String,
    pub registry_key: String,
    pub registry_owner_keypair: Option<zeroize::Zeroizing<String>>,
    pub slot_seed_hex: String,
    pub slot_keypair: String,
    pub my_pseudonym_hex: String,
    pub mek: MekSnapshot,
    pub governance_state: GovernanceState,
    /// The pinned `GenesisCert` as `A`/0 holds it
    /// (`rekindle_codec::governance_op::genesis::encode_genesis`), stored
    /// in `communities.genesis_cert`.
    pub genesis_cert: Vec<u8>,
    /// Our member record `M(p)`'s key.
    pub member_record: String,
    /// `K_M(p)`'s keypair text: stored as
    /// `VaultKey::MemberRecordOwner{community}` before the row (D1).
    pub member_record_owner_keypair: zeroize::Zeroizing<String>,
    /// Our card's X25519 HPKE secret: stored as
    /// `VaultKey::MemberHpkeSecret{community}` before the row, so before
    /// the card that carries its public half.
    pub member_hpke_secret: zeroize::Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for CommunityInsert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommunityInsert")
            .field("id", &self.id)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// What the publisher of a governance op waits for
/// ([`super::GovernanceRuntimeDeps::publish_governance_op`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpLanding {
    /// The first attempt, at most `GOVERNANCE_ANSWER_BOUND`: a queued op
    /// stands, the outbox lands it, and the answer is `Pending`.
    FirstAttempt,
    /// The op on `M`/2 and our frontier on `M`/3, through the retries: only
    /// for what a joiner must find on the DHT before we hand it out, an
    /// invite's `InviteCreated` (S6 (4)). Stored in one transaction with a
    /// revoke of `invite_id` that the outbox writes once the op landed
    /// unless the answer handed the link out (review RV4 M3). `deadline`
    /// bounds the wait in line and the landing together.
    InviteLanded {
        invite_id: [u8; 16],
        deadline: tokio::time::Instant,
    },
    /// Queued on the stream, not waited for: the outbox lands it behind our
    /// earlier ops (one stream per author, so the order holds).
    Queued,
}

impl OpLanding {
    /// An invite's `InviteCreated`, answered within the command's network
    /// deadline from now (`NETWORK_ANSWER_DEADLINE`).
    #[must_use]
    pub fn invite_landed(invite_id: [u8; 16]) -> Self {
        let bound = rekindle_types::config::NETWORK_ANSWER_DEADLINE;
        Self::InviteLanded {
            invite_id,
            deadline: tokio::time::Instant::now() + bound,
        }
    }

    /// What is left of an invite's deadline (zero past it); the other
    /// landings carry no deadline, so zero.
    #[must_use]
    pub fn remaining(self) -> std::time::Duration {
        match self {
            Self::InviteLanded { deadline, .. } => {
                deadline.saturating_duration_since(tokio::time::Instant::now())
            }
            Self::FirstAttempt | Self::Queued => std::time::Duration::ZERO,
        }
    }
}

/// How a published governance op stands when its command answers (plan
/// E3.2 S6b): both are successes, since the op is applied, gossiped and
/// queued and nothing cancels it.
/// Our op's row on the governance-op stream, stored
/// ([`GovernanceRuntimeDeps::publish_governance_op`](crate::deps::GovernanceRuntimeDeps::publish_governance_op)
/// returns it once the row is durable), and the wait for what its
/// [`OpLanding`] names (lead ruling 2026-11-26, item 2: the row exists
/// before the op is gossiped, so no abort between the two loses it).
pub struct OpPublication(
    std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<OpOutcome, crate::error::GovernanceRuntimeError>,
                > + Send,
        >,
    >,
);

impl OpPublication {
    /// The wait `landed` for a row already stored.
    pub fn new(
        landed: impl std::future::Future<Output = Result<OpOutcome, crate::error::GovernanceRuntimeError>>
            + Send
            + 'static,
    ) -> Self {
        Self(Box::pin(landed))
    }

    /// A publication whose outcome is already known.
    #[must_use]
    pub fn settled(outcome: Result<OpOutcome, crate::error::GovernanceRuntimeError>) -> Self {
        Self::new(std::future::ready(outcome))
    }

    /// Wait for what the op's landing names.
    ///
    /// # Errors
    /// As the landing it waits for.
    pub async fn landed(self) -> Result<OpOutcome, crate::error::GovernanceRuntimeError> {
        self.0.await
    }
}

impl std::fmt::Debug for OpPublication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpPublication")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpOutcome {
    /// On our `M`/2 with our frontier on `M`/3.
    Landed,
    /// Queued on the governance-op stream; the outbox lands it, and the
    /// backend announces the landing.
    Pending,
    /// Held behind the unlock barrier (plan E3.2 S7d-keys.8), unsigned:
    /// listed under `op_id` in the community's `CatchUp` pending list, it
    /// is signed in the background once the barrier passed, and its
    /// landing or refusal reaches the window as any op's.
    Held { op_id: String },
}

impl OpOutcome {
    /// The held write's id in the `CatchUp` pending list, when held.
    #[must_use]
    pub fn held_op_id(&self) -> Option<&str> {
        match self {
            Self::Held { op_id } => Some(op_id),
            Self::Landed | Self::Pending => None,
        }
    }
}

/// What a member's verified card (`M`/0) says that the community renders
/// (plan E3.4 S6): its per-community name (`None` = unset or empty; readers
/// show the short pseudonym fingerprint) and its custom status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardText {
    pub display_name: Option<String>,
    pub custom_status: Option<String>,
    /// The game the member shares there (plan E3.4 S6, field-fate table:
    /// `game_info` is the card's); `None` while they share none.
    pub game: Option<rekindle_types::presence::GameInfo>,
}
