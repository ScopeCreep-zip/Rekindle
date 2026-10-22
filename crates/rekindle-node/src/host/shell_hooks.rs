//! The shell's hooks (plan E1.1, ADR 0020): where the Host waits on the
//! GUI's not-yet-ported stages during an unlock and its teardown. The Host
//! owns the lifecycle; the GUI implements this with its existing code
//! (`src-tauri/src/services/shell_session.rs`), each domain port deletes
//! its part, and F1 deletes the trait. `rekindled` has no hooks.
//!
//! The shape is the delegate pattern of core-plus-GUI apps (Keybase's
//! service-called UI protocols, matrix-rust-sdk's delegate callbacks): the
//! core calls the frontend at fixed points and waits only where it must —
//! the shell's unported unlock stages run inside the Host's unlock scope
//! and can abort the unlock, and its leave of calls runs before the scope
//! is cancelled (C7.27).

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use zeroize::Zeroizing;

/// What an unlock hands the shell: the identity and the keys its unported
/// stages still read.
pub struct UnlockedView {
    pub public_key: String,
    pub display_name: String,
    /// The Ed25519 identity secret.
    pub secret: Zeroizing<[u8; 32]>,
}

/// The identity's own records, once the own-records stage has them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnRecordKeys {
    pub profile: Option<String>,
    pub friend_list: Option<String>,
    pub mailbox: Option<String>,
}

/// Why an unlock ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// `Lock`.
    Lock,
    /// `Logout`, or a rotation that moved the identity to a new key.
    Logout,
    /// The unlocked identity was deleted or wiped.
    Deleted,
    /// The host is stopping.
    Exit,
    /// The unlock failed after it began; nothing it started is left.
    Failed,
    /// A task of the unlock panicked (its name).
    TaskPanicked(String),
}

/// The GUI's side of an unlock and its end.
#[async_trait::async_trait]
pub trait ShellHooks: Send + Sync {
    /// The identity is installed and the Host's stages are running: start
    /// the shell's unported stages as children of `scope`. An error aborts
    /// the unlock through the teardown.
    ///
    /// # Errors
    /// A stage could not start.
    async fn unlocked(&self, view: &UnlockedView, scope: &Arc<SessionScope>) -> Result<(), String>;

    /// The own-records stage finished: the keys it holds.
    fn own_records(&self, keys: &OwnRecordKeys);

    /// The teardown is starting, before the unlock scope is cancelled: what
    /// must still send while the session can.
    async fn ending(&self);

    /// The teardown finished; the identity is out of the Host.
    async fn ended(&self, reason: EndReason);

    // ---- Governance capabilities the Host's adapter cannot reach yet
    // (plan E3.1 B2). `rekindled` runs its own; each owner step deletes one.

    /// Fan a governance envelope out over the shell's gossip mesh; an error
    /// when it could not go. Ported with the gossip mesh at E3.4.
    ///
    /// # Errors
    /// The envelope did not reach the mesh.
    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), String>;

    /// Start the shell's history catch-up for a community. Ported with the
    /// catch-up at E3.1 C2.
    fn spawn_history_catchup(&self, community_id: &str);

    /// Open the community's Lost Cargo chunk cache. Ported with the files
    /// domain at E3.1 E3.
    fn ensure_files_cache_open(&self, community_id: &str);

    /// Start the key rotations a departure (a ban) requires. Ported with
    /// the MEK adapters at E3.5.
    fn spawn_departure_rotation(&self, community_id: &str, departed_pseudonym_hex: &str);

    /// The shell's permission check over its own role mirror. Ported when
    /// the Host's `community_membership` reads the community store (E3.1 B2).
    ///
    /// # Errors
    /// We lack the permission, or the community is unknown.
    fn require_permission(&self, community_id: &str, perm_bits: u64) -> Result<(), String>;

    /// The Host projected merged governance onto the community
    /// (`adapters::governance::projection`): the shell's effects of a merge
    /// that the Host cannot run yet. Each names its owner step: the Lost
    /// Cargo cache and pinned set, and the eager expression fetch (the files
    /// domain, E3.1 E3); opening new Plate Gate segment records and the
    /// watch refresh when `projected.new_watch_targets` (the watch path,
    /// E3.1 C1); the event RSVP lists, the `community_events` rows and the
    /// reminder wake (the events domain, E3.1 E3).
    fn on_governance_projected(
        &self,
        community_id: &str,
        projected: &crate::host::adapters::governance::projection::GovernanceProjected,
    );
}
