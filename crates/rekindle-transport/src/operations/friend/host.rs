//! What the friend and 1:1 operations need from the Host (plan E2.1). The
//! transport sits below `rekindle-node`, so everything the Host owns comes
//! through this one trait, which the Host implements once for both hosts
//! (`rekindle_node::host::adapters::friends`).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_events::typing::{TypingExpiry, TypingSessions};
use rekindle_types::friend_store::FriendStore;
use rekindle_types::subscription_events::SubscriptionEvent;
use zeroize::Zeroizing;

use super::error::FriendError;
use super::session_reset::IncomingReset;
use crate::broadcast::node::TransportNode;
use crate::operations::direct::replay::ReplayGuard;
use crate::session::Session;

/// An action the backend holds for the user's review (plan E2.1): the Host
/// turns it into its gate's held action and the one sent on Cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendHold {
    /// A friend asked to reset our secure session: commit accepts it,
    /// Cancel declines it.
    SessionReset { peer: String },
    /// A friend rotated their identity to `new_identity` (hex): commit
    /// moves them to it, Cancel drops the rotation (owner ruling R1).
    IdentityRotation { peer: String, new_identity: String },
}

/// The 1:1 receive and handshake state the Host keeps for the unlocked
/// identity, cleared at lock.
#[derive(Default)]
pub struct FriendStores {
    /// Pre-session frames already accepted, so a replayed copy is dropped.
    pub replay: Mutex<ReplayGuard>,
    /// Each friend's pending reset request, until the user answers it (one
    /// per friend, the newest wins).
    pub incoming_resets: Mutex<HashMap<String, IncomingReset>>,
    /// Our first contacts not yet answered under their session, by peer
    /// (plan E2.4, B6).
    pub first_contacts: Mutex<HashMap<String, crate::operations::prekeys::FirstContact>>,
    /// Each friend's verified identity rotation awaiting the user's
    /// review, by their current key: the new identity (owner ruling R1).
    /// Memory only; the friend re-sends it on their next contact.
    pub pending_rotations: Mutex<HashMap<String, [u8; 32]>>,
    /// Who is typing to us, and when each start lapses (plan E2.3).
    pub typing_in: Mutex<TypingExpiry>,
    /// Our typing sessions, one per conversation (plan E2.3).
    pub typing_out: Mutex<TypingSessions>,
}

impl FriendStores {
    /// Forget everything (lock).
    pub fn clear(&self) {
        self.replay.lock().clear();
        self.incoming_resets.lock().clear();
        self.first_contacts.lock().clear();
        self.pending_rotations.lock().clear();
        self.typing_in.lock().clear();
        self.typing_out.lock().clear();
    }
}

/// The Host, as the friend and 1:1 operations see it.
#[async_trait::async_trait]
pub trait FriendHost: Send + Sync {
    /// An owned handle to this Host, for work that outlives the call (a
    /// background send in the unlock scope).
    fn owned(&self) -> Arc<dyn FriendHost>;
    /// The unlocked identity's database.
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn db(&self) -> Result<rekindle_db::Db, FriendError>;
    /// The session projection (`HostStores.session`).
    fn session(&self) -> &Arc<RwLock<Option<Session>>>;
    /// Our side of a ratchet step: the unlocked identity, the Host's
    /// generator and its clock now (plan E2.3, `rekindle_db::ratchet`).
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn local_party(&self) -> Result<rekindle_db::ratchet::LocalParty, FriendError>;
    /// The Ed25519 identity secret.
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn identity_secret(&self) -> Result<Zeroizing<[u8; 32]>, FriendError>;
    /// The transport node: its ports, records and peer route cache.
    ///
    /// # Errors
    /// `NotReady` before the node is up.
    fn node(&self) -> Result<Arc<TransportNode>, FriendError>;
    /// The receive gate's friendship authority over the identity's
    /// database.
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn friend_store(&self) -> Result<Arc<dyn FriendStore>, FriendError>;
    /// The 1:1 receive and handshake state.
    fn stores(&self) -> &FriendStores;
    /// Journal an event for every frontend (D21).
    fn publish(&self, event: SubscriptionEvent);
    /// Append to the identity's audit chain.
    async fn audit(&self, kind: rekindle_audit::AuditKind, detail: serde_json::Value);
    /// Watch a friend's profile record for their presence.
    async fn watch_friend(&self, peer: &str, profile_dht_key: &str);
    /// Stop watching a friend's profile record (removal, block, rotation).
    async fn unwatch_friend(&self, peer: &str);
    /// Move our profile to a new record under a fresh owner key (block),
    /// as the Host's own-records stage writes one; the new record's key.
    ///
    /// # Errors
    /// The new record could not be created or stored.
    async fn rotate_own_profile(&self) -> Result<String, FriendError>;
    /// Hold `held` for the user's review.
    ///
    /// # Errors
    /// The gate refused it (too many held for this identity).
    async fn hold_backend_intent(&self, held: BackendHold) -> Result<(), FriendError>;
    /// Our prekey record's writable lease, once the unlock's own-records
    /// stage opened or created it (plan E2.4).
    fn own_prekey_lease(&self) -> Option<rekindle_records::lease::LeaseId>;
    /// The unlock's scope: background sends run in it.
    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope>;
    /// The Host's clock (C9.2).
    fn clock(&self) -> &dyn rekindle_utils::Clock;
    /// The Host's id and nonce source (C9.2); never an AEAD nonce.
    fn rng(&self) -> &dyn rekindle_utils::RandomSource;
}
