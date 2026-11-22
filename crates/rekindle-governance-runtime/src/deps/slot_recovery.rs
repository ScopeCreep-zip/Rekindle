//! Our member slot and its keys, recovered at login: the slot model's
//! hydration (`dht_hydration::hydrate_one_community`), one responsibility
//! of the runtime's deps, reached through
//! [`GovernanceRuntimeDeps::slot_recovery`](super::GovernanceRuntimeDeps::slot_recovery).

/// The login-time recovery of our registry slot, its slot keypair and the
/// registry owner keypair.
pub trait SlotRecovery: Send + Sync {
    /// Snapshot every joined community's
    /// `(community_id, registry_key, my_pseudonym_hex_opt)`. Communities
    /// without a `member_registry_key` are skipped. Used by
    /// `dht_hydration::hydrate_one_community` to enumerate
    /// records to read from the DHT.
    fn list_registries_with_my_pseudonym(&self) -> Vec<(String, String, Option<String>)>;

    /// Apply recovered member state for one community: adopts
    /// `my_subkey_index` and persists it to SQLite so the next login can
    /// skip the recovery step. Our roles are the merged role assignments
    /// (the projection), never recovered here.
    fn apply_recovered_member_state(&self, community_id: &str, subkey_index: u32);

    /// If the community has a `slot_seed` + `my_subkey_index` but no
    /// `slot_keypair`, derive the keypair now via the existing
    /// `services::community::try_derive_slot_keypair`. No-op otherwise.
    fn try_derive_slot_keypair_if_ready(&self, community_id: &str);

    /// Belt-and-suspenders: list communities whose
    /// `registry_owner_keypair` is empty. Used by the orchestrator to
    /// trigger Stronghold recovery for races where login didn't load
    /// the keypair in time.
    fn list_missing_registry_keypairs(&self) -> Vec<String>;

    /// Look up the registry owner keypair from Stronghold and install
    /// it on the community's state. No-op when the keystore lookup
    /// returns `None`.
    fn recover_registry_keypair_from_keystore(&self, community_id: &str);
}
