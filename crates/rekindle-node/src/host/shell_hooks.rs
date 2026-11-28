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

    /// The Host signed a gossip envelope of ours under `dedup_key`: the
    /// shell records it in the cache its inbound gossip is metered against,
    /// so a relayed copy coming back is dropped as a duplicate. Ported with
    /// the inbound gossip path.
    fn own_gossip_signed(&self, community_id: &str, sender: &str, dedup_key: &str);

    /// We left the community: drop the shell's own copies of its keys,
    /// until the MEK adapters move at E3.5.
    fn community_left(&self, community_id: &str);

    /// Start the key rotations a departure (a ban) requires. Ported with
    /// the MEK adapters at E3.5.
    fn spawn_departure_rotation(&self, community_id: &str, departed_pseudonym_hex: &str);

    /// The Host projected merged governance onto the community
    /// (`adapters::governance::projection`): the shell's effects of a merge
    /// that the Host cannot run yet (the Host itself syncs the pinned set
    /// and fetches the expressions, E3.1 E3f). Each names its owner step:
    /// opening new Plate Gate segment records and the watch refresh when `projected.new_watch_targets` (the watch path,
    /// E3.1 C1); the event RSVP lists, the `community_events` rows and the
    /// reminder wake (the events domain, E3.1 E3).
    fn on_governance_projected(
        &self,
        community_id: &str,
        projected: &crate::host::adapters::governance::projection::GovernanceProjected,
    );

    /// Re-acquire the community key we hold behind: the shell's retrying
    /// request over its own key cache, until the MEK adapters move at
    /// E3.5. It never mints a key (FX3).
    fn spawn_community_mek_recovery(&self, community_id: &str, requester_pseudonym: &str);

    /// A watched registry of the community changed: the Host decided on a
    /// refresh (read and merge the rows, never write ours, plan E3.4 S5,
    /// finding IB1c); the shell runs it over its own presence adapter until
    /// the polls move into the Host (S8b).
    async fn presence_refresh(&self, community_id: &str);

    /// Mark our onboarding of the community complete in the personal sync
    /// record, so paired devices stop showing the wizard. Ported with
    /// cross-device sync at E5.2.
    fn push_onboarding_complete_to_sync(&self, community_id: &str);

    /// A community or channel key the Host's cache took: the shell's own key
    /// cache, which its channel paths read, until the MEK adapters move at
    /// E3.5.
    fn on_mek_installed(
        &self,
        community_id: &str,
        scope: rekindle_types::channel_keys::KeyScope,
        key_bytes: [u8; 32],
        generation: u64,
    );

    /// The shell's channel-key reader over its own key cache, which its
    /// gossip, channel and rotation paths fill: the governance runtime reads
    /// it (a joiner's bootstrap bundle, a rotation's prior key) until the MEK
    /// adapters move at E3.5.
    fn channel_keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider>;

    /// The shell's voice state (plan E3.4 S8-pre, the `channel_keys()`
    /// pattern): its voice session's channel, which our beat carries, and
    /// its voice roster, which the members' beats feed; nothing else (our
    /// session and consent are the Host's). E4.1 deletes it when voice
    /// becomes the Host's.
    fn session_presence(&self) -> Arc<dyn rekindle_presence::SessionPresence>;

    /// The governance runtime handed over a community's session leases:
    /// the shell holds them, watches the records and starts the community's
    /// inspect, presence and keepalive loops once. Ported when the leases
    /// and keepalive become both-mode and the presence polls move with
    /// presence at E3.4.
    async fn community_records_ready(
        &self,
        community_id: &str,
        leases: rekindle_records::lease::CommunityLeases,
    );
}
