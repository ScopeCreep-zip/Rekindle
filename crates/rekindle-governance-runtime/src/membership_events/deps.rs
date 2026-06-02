//! Deps surface for the legacy membership-event drain (Phase 23.E).
//!
//! These handlers interpret incoming community **control messages**
//! (JoinAccepted, keypair grants, role changes, onboarding answers,
//! peer-assisted join) and need AppState mutation + SQLite + Stronghold +
//! Veilid DHT capabilities. Those live behind this sub-trait, implemented
//! by `GovernanceAdapter` at
//! `src-tauri/src/services/governance_adapter/membership_events.rs`.
//!
//! `MembershipEventDeps: GovernanceRuntimeDeps` (supertrait) so the
//! onboarding handler can call `apply::write_entry` / `send_to_mesh` /
//! `governance_state` without re-declaring them here.

use async_trait::async_trait;

use crate::deps::GovernanceRuntimeDeps;

/// One member row to upsert into `community_members` from a
/// `JoinAccepted` member list.
#[derive(Debug, Clone)]
pub struct MemberUpsertRow {
    pub pseudonym_hex: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub joined_at: u64,
    pub subkey_index: u32,
    pub onboarding_complete: bool,
    pub timeout_until: Option<u64>,
}

/// Material to install after a keypair / slot-seed grant is unwrapped
/// in-crate. Every field is optional so one struct covers admin-keypair
/// grants (owner keypair + seed), slot-keypair grants (keypair + index)
/// and the JoinAccepted slot-seed path (seed + keypair + index).
#[derive(Debug, Clone, Default)]
pub struct SlotGrantUpdate {
    pub slot_seed_hex: Option<String>,
    pub slot_keypair: Option<String>,
    pub dht_owner_keypair: Option<String>,
    pub my_subkey_index: Option<u32>,
}

/// I/O capabilities the membership-event handlers need beyond what
/// `GovernanceRuntimeDeps` already provides.
#[async_trait]
pub trait MembershipEventDeps: GovernanceRuntimeDeps {
    // ---------- Roles ----------

    /// Replace `role_ids` for a member in `community_members`; if
    /// `is_self`, also update `community.my_role_ids` (in-memory + the
    /// `communities` row). Fire-and-forget SQLite write (`db_fire`).
    fn persist_member_roles(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    );

    // ---------- JoinAccepted ----------

    /// Set `mek_generation` + `member_registry_key` on the in-memory
    /// `CommunityState` and persist the registry key to the
    /// `communities` row.
    async fn set_mek_generation_and_registry(
        &self,
        community_id: &str,
        mek_generation: u64,
        member_registry_key: Option<&str>,
    );

    /// `INSERT OR REPLACE` every row into `community_members`.
    async fn upsert_members(&self, community_id: &str, members: Vec<MemberUpsertRow>);

    /// Set `my_subkey_index` from the member list when it's currently
    /// unset (in-memory, always). If `persist`, also write it to the
    /// `communities` row — the JoinAccepted backup path used when the
    /// accept carried no explicit `slot_index` (legacy join.rs:231-267).
    /// Fire-and-forget SQLite write (`db_fire`).
    fn backfill_my_subkey_index(&self, community_id: &str, subkey_index: u32, persist: bool);

    /// Add pseudonyms to the in-memory `known_members` set.
    fn insert_known_members(&self, community_id: &str, pseudonyms: &[String]);

    /// Persist the currently-cached community MEK to Stronghold.
    fn persist_community_mek_to_keystore(&self, community_id: &str);

    /// Fire-and-forget: open the registry, read each member's presence
    /// row, sig-verify it, and seed the gossip overlay with online peers
    /// (the legacy `spawn_peer_bootstrap` loop — all Veilid + gossip).
    fn spawn_peer_bootstrap(&self, community_id: &str, members: Vec<MemberUpsertRow>);

    // ---------- Grants / slot seed ----------

    /// Install unwrapped grant material into AppState, Stronghold and
    /// SQLite in one shot (only the `Some` fields are written).
    fn apply_slot_grant(&self, community_id: &str, update: SlotGrantUpdate);

    /// Fire-and-forget: run a single immediate presence-poll tick to
    /// bring the gossip overlay up-to-date right after a slot grant —
    /// *not* the recurring cadence loop (which the join flow already
    /// owns). Mirrors the legacy one-shot `presence_poll_tick_public`
    /// spawn in the grant handlers.
    fn spawn_presence_poll_tick(&self, community_id: &str);

    // ---------- Onboarding ----------

    /// Set `member_roles[pseudonym] = role_ids`; if `is_self`, flip
    /// `onboarding_complete = true` (in-memory + the `community_members`
    /// row). Fire-and-forget SQLite write (`db_fire`).
    fn persist_onboarding_completion(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        role_ids: &[u32],
        is_self: bool,
    );

    /// Fire-and-forget: push our own onboarding completion into the
    /// personal SMPL ReadState so paired devices stop showing the wizard
    /// (architecture §28.4).
    fn push_onboarding_complete_to_sync(&self, community_id: &str);

    // ---------- Peer-assisted join ----------

    /// Increment the local uses counter for a redeemable invite code and
    /// emit `InviteUsed` when our row matches (architecture §16).
    fn bump_invite_uses(&self, community_id: &str, invite_code: &str);
}
