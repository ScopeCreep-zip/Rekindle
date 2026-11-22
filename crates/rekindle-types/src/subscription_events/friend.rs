//! Friend lifecycle events — request, accept, reject, remove, profile rotation.
//!
//! These are triggered by 1:1 kinds arriving as `'D'` envelopes on the
//! identity's own route.

use serde::{Deserialize, Serialize};

/// Friend lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FriendEvent {
    /// An inbound friend request was received.
    /// Triggered by: an inbound 1:1 `FriendRequest`.
    RequestReceived {
        from_key: String,
        display_name: String,
        message: String,
    },
    /// A friend request we sent was acknowledged (delivery confirmed).
    /// Triggered by: the receipt for our 1:1 `FriendRequest`.
    RequestAcknowledged { peer_key: String },
    /// A friend request was accepted (friendship established).
    /// Triggered by: an inbound 1:1 `FriendAccept`.
    Accepted {
        peer_key: String,
        /// The accepter's display name, when the acceptance carried it.
        /// The desktop's parallel event always had this and Tier 1 did
        /// not, so a CLI saw an acceptance from a bare key.
        display_name: Option<String>,
    },
    /// A friend request was rejected.
    /// Triggered by: an inbound 1:1 `FriendReject`.
    Rejected { peer_key: String },
    /// A friend row was added to our list — the point at which the
    /// friend becomes renderable, as distinct from [`Self::Accepted`],
    /// which is the protocol acknowledgement that precedes it.
    Added {
        peer_key: String,
        display_name: String,
        friendship_state: crate::friend_store::FriendshipState,
        /// The group the buddy list shows them under
        /// ([`crate::dto::friends::group_label`]).
        group_label: String,
    },
    /// A friend removed us.
    /// Triggered by: an inbound 1:1 `Unfriended`.
    Removed { peer_key: String },
    /// Unfriend was acknowledged.
    /// Triggered by: an inbound 1:1 `UnfriendedAck`.
    RemoveAcknowledged { peer_key: String },
    /// A friend rotated their profile DHT key.
    /// Triggered by: an inbound 1:1 `ProfileKeyRotated`.
    ProfileKeyRotated {
        peer_key: String,
        new_profile_dht_key: String,
    },
    /// We set or cleared our local nickname for a friend.
    /// Triggered by: the `set_friend_nickname` action.
    NicknameChanged {
        peer_key: String,
        nickname: Option<String>,
    },
    /// A friend's own display name changed, as their profile record holds
    /// it (subkey 0, live A/B finding 39).
    /// Triggered by: the read after their profile is watched, and a value
    /// change of its subkey 0.
    DisplayNameChanged {
        peer_key: String,
        display_name: String,
    },
    /// A friend's bio (their `ProfileBio`) replaced the one we held; a
    /// window showing their profile reads it again (`ProfileGet`).
    /// Triggered by: an accepted friend's `ProfileBio`, once stored.
    BioChanged { peer_key: String },
    /// A friend's group changed: moved, or their group renamed or deleted
    /// (`None`: no group).
    /// Triggered by: the friend-group actions.
    GroupChanged {
        peer_key: String,
        /// The id of their group now, the one the group actions take.
        group_id: Option<i64>,
        group: Option<String>,
        /// The group the buddy list shows them under now
        /// ([`crate::dto::friends::group_label`]).
        group_label: String,
    },
    /// A friend rotated their identity: they are `new_key` now, with their
    /// history and settings moved, and unverified (plan E2.3).
    /// Triggered by: their signed `IdentityRotated` frame.
    KeyChanged { peer_key: String, new_key: String },
    /// The first-contact Signal handshake with a peer we sent a request to
    /// failed: no secure session exists, so the friendship stays pending.
    /// This is not a trust prompt; it never asks for "Reset Secure
    /// Session" (plan C7.25). An identity-key mismatch also lands here,
    /// with its own notice.
    /// Triggered by: a `FriendAccept` whose session init does not complete.
    HandshakeFailed { peer_key: String, reason: String },
    /// The peer rejected the one-time key our first contact used (it was
    /// already taken): the handshake is resent once from a fresh key
    /// (plan E2.4, D18's B6). The friend row reads "Retrying with a fresh
    /// key" until the session opens or [`Self::HandshakeFailed`].
    /// Triggered by: their signed `PreKeyRejected`.
    HandshakeRetrying { peer_key: String },
    /// Whether the peer's route answers (plan RH1r-c S4): after six suspect
    /// rounds without an answer the row reads "unreachable"; messages still
    /// go through the durable chain. Any authenticated frame from the peer
    /// makes it reachable again.
    /// Triggered by: the sender's route liveness rules.
    Reachability { peer_key: String, reachable: bool },
    /// A friend's prekey record is not signed by the identity we hold for
    /// them, so no session was opened and nothing was trusted; only the
    /// user can resolve it (plan E2.4, B3).
    /// Triggered by: a record read before a first contact or a send.
    IdentityPinMismatch {
        peer_key: String,
        display_name: String,
    },
    /// A friend whose record had failed the identity check verifies again:
    /// the alert on their row ends and sends are allowed (plan E2.4, B3).
    /// Triggered by: their record read after an `IdentityPinMismatch`.
    IdentityPinRestored { peer_key: String },
    /// We marked a peer's identity key verified, or cleared it (plan E2.3):
    /// every window showing their badge or safety number re-renders.
    /// Triggered by: the `IdentityMarkVerified` / `IdentityClearVerification`
    /// actions.
    VerificationChanged { peer_key: String, verified: bool },
    /// We blocked or unblocked an identity: the block list changed. A block
    /// also removes the friend ([`Self::Removed`]).
    /// Triggered by: the `FriendBlock` / `FriendUnblock` actions.
    BlockChanged { peer_key: String, blocked: bool },
    /// An accepted friend invites us to join them on a game server (plan
    /// E7.2). `server` is a parsed `ServerAddress` (host:port), shown as is;
    /// joining is a separate, reviewed action.
    /// Triggered by: an inbound 1:1 `GameInvite` (0x0170).
    GameInvite {
        peer_key: String,
        game_id: u32,
        server: crate::game_server::ServerAddress,
    },
    /// An accepted friend names us a co-creator of `community_name`,
    /// reserved at `anchor` (plan E3.2 S7s.3); held until we answer or the
    /// friend withdraws it. Triggered by: an inbound `CreatorInvite`
    /// (0x0190), and replayed from `creator_invites` at unlock.
    CreatorInvited {
        peer_key: String,
        anchor: String,
        community_name: String,
    },
    /// The friend withdrew its co-creator invite to `anchor` (removed us,
    /// or its creation died before its genesis, decision (20)).
    /// Triggered by: an inbound `CreatorWithdrawn` (0x0193).
    CreatorInviteWithdrawn { peer_key: String, anchor: String },
    /// A co-creator's state in our creation of `anchor` (plan E3.2 S7s.3):
    /// `pending`, `accepted`, `declined`, `removed` or `withdrawn`.
    /// Triggered by: our create naming it, its `CreatorAccept` (0x0191) or
    /// `CreatorDecline` (0x0192), our removal, or the unlock sweep.
    CoCreatorStatus {
        peer_key: String,
        anchor: String,
        status: String,
    },
    /// Our creation of `anchor` with co-creators ended (plan E3.2 S7s.3g,
    /// lead decision (33)): `created` when its genesis is written (the
    /// community's id is `anchor`), otherwise it failed and every accepted
    /// co-creator was withdrawn. One event for the creation, not per
    /// co-creator: a creation every co-creator declined has no peer to
    /// carry a terminal `CoCreatorStatus`. Triggered by: the last
    /// co-creator answering or being removed (`co_creator_settled`).
    CoCreationFinished { anchor: String, created: bool },
}
