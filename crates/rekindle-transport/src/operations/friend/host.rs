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
    /// Our retry requests to a friend are exhausted (plan E2.5 last stage,
    /// ruling Q1): commit re-initiates the session with the user-started
    /// reset, Cancel only drops the hold. Nothing re-initiates without it.
    SessionReinitiate { peer: String },
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
    /// Our group control frames that opened a session with a roster member
    /// from their prekey record, until they answer under it (plan E2.6,
    /// B6): the frame resent once from a fresh slot if they refuse it.
    pub group_first_contacts:
        Mutex<HashMap<String, crate::operations::group_dm::GroupFirstContact>>,
    /// Each friend's verified identity rotation awaiting the user's
    /// review, by their current key: the new identity (owner ruling R1).
    /// Memory only; the friend re-sends it on their next contact.
    pub pending_rotations: Mutex<HashMap<String, [u8; 32]>>,
    /// Who is typing to us, and when each start lapses (plan E2.3).
    pub typing_in: Mutex<TypingExpiry>,
    /// Our typing sessions, one per conversation (plan E2.3).
    pub typing_out: Mutex<TypingSessions>,
    /// Delivery receipts waiting for their coalescing delay, or held while
    /// we are Invisible (plan E2.5).
    pub acks: Mutex<crate::operations::dm::ack::PendingAcks>,
    /// The DM chain diffs in flight, one per peer (plan E2.5, D23).
    pub streams: rekindle_sync::stream::StreamEngine,
    /// One gate per peer around our participant record's check-and-create
    /// (plan E2.5, MSR): one creator, the others await its record.
    pub participant_gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl FriendStores {
    /// Forget everything (lock).
    pub fn clear(&self) {
        self.replay.lock().clear();
        self.incoming_resets.lock().clear();
        self.first_contacts.lock().clear();
        self.group_first_contacts.lock().clear();
        self.pending_rotations.lock().clear();
        self.typing_in.lock().clear();
        self.typing_out.lock().clear();
        self.acks.lock().clear();
        self.participant_gates.lock().clear();
    }

    /// The gate around our participant record for `peer`.
    #[must_use]
    pub fn participant_gate(&self, peer: &str) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(
            self.participant_gates
                .lock()
                .entry(peer.to_string())
                .or_default(),
        )
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
    /// The unlocked identity's outbox (plan E2.5, D22).
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn outbox(&self) -> Result<Arc<dyn crate::operations::dm::outbox::DmOutbox>, FriendError>;
    /// Our dm participant record for the conversation with `peer` and its
    /// owner keypair (plan E2.5, MSR), created when none is stored: the
    /// keypair is saved to the vault before the record key is stored.
    async fn participant_writer(
        &self,
        peer: &str,
    ) -> Result<crate::operations::dm::stream::ParticipantWriter, FriendError>;
    /// Our stored participant record for `peer` and its saved writer, when
    /// both are held; never creates one.
    async fn held_participant_writer(
        &self,
        peer: &str,
    ) -> Result<Option<crate::operations::dm::stream::ParticipantWriter>, FriendError>;
    /// The group-DM owner's Host side over the unlocked identity (plan
    /// E2.6): its groups, keys and held invites, identity signature and
    /// event journal.
    ///
    /// # Errors
    /// `Locked` while no identity is unlocked.
    fn group_dm(&self) -> Result<Arc<dyn rekindle_dm::GroupDmDeps>, FriendError>;
    /// Our group chain for `group_id` (32 hex) and its owner keypair
    /// (plan E2.6), made single-flight when none is saved; the keypair is
    /// in the vault before the key is returned.
    async fn group_chain_writer(
        &self,
        group_id: &str,
    ) -> Result<crate::operations::dm::stream::ParticipantWriter, FriendError>;
    /// Whether our own status is Invisible (receipts are held meanwhile).
    fn invisible(&self) -> bool;
    /// Journal an event for every frontend (D21).
    fn publish(&self, event: SubscriptionEvent);
    /// Append to the identity's audit chain.
    async fn audit(&self, kind: rekindle_audit::AuditKind, detail: serde_json::Value);
    /// Watch a friend's profile record for their presence.
    async fn watch_friend(&self, peer: &str, profile_dht_key: &str);
    /// Watch the participant record `record_key` that `peer`'s chain to us
    /// is on (plan E2.5): a record replacing the one held re-watches.
    async fn watch_participant(&self, peer: &str, record_key: &str);
    /// Stop watching a friend's profile record (removal, block, rotation).
    async fn unwatch_friend(&self, peer: &str);
    /// Hold an invite's request inbox (plan E2.5, D17): keep its owner
    /// keypair in the vault, and watch its writer's subkey on `lease`.
    async fn keep_request_inbox(
        &self,
        record_key: &str,
        owner_keypair: &str,
        lease: rekindle_records::lease::LeaseId,
    ) -> Result<(), FriendError>;
    /// Drop an invite's request inbox: stop watching it, release it and
    /// forget its owner keypair.
    async fn drop_request_inbox(&self, record_key: &str);
    /// Move our profile to a new record under a fresh owner key (block),
    /// as the Host's own-records stage writes one; the new record's key.
    ///
    /// # Errors
    /// The new record could not be created or stored.
    async fn rotate_own_profile(&self) -> Result<String, FriendError>;
    /// Rewrite our STATUS and GAME_INFO under the session's current
    /// friend-presence key (after a rotation, plan E2.5).
    async fn reseal_own_presence(&self);
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
