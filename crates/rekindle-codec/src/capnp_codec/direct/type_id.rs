//! The `DmFrame` typeIds (§6 `DmFrame` typeIds; allocated there only) and
//! how each kind travels.

/// How a kind travels in a `DmFrame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carrier {
    /// The PQXDH init itself (0x0001): a `preKeyInit` body.
    PreKeyInit,
    /// Only as the encrypted first payload of a PreKeyInit, never as a
    /// frame's own typeId (FriendAccept, SessionResetAccept).
    FirstPayload,
    /// HPKE Base to the recipient identity plus the sender's identity
    /// signature: a `sealed` body (the pre-session kinds).
    Sealed,
    /// The Double Ratchet session: a `ratchet` body, or the first payload
    /// of a PreKeyInit that opens a session.
    Ratchet,
    /// Never a `DmFrame`: an entry of the sender's group chain (plan E2.6),
    /// its `sealed` already the AEAD under the sender's group sending key,
    /// which every member holds. The chain is read by every member, so a
    /// pairwise ratchet or HPKE seal (one reader) cannot carry it, and
    /// sealing it again per member would write one entry per reader; the
    /// members' hints carry the same entry over each pairwise ratchet,
    /// only to make the member diff the chain (typing, which is never on a
    /// chain, travels only so). As a frame it takes a `ratchet` body, never
    /// a sealed or PreKeyInit one.
    GroupChain,
}

macro_rules! type_ids {
    ($( $(#[$doc:meta])* $kind:ident = $value:literal => $carrier:ident, )*) => {
        /// A `DmFrame` kind, by its §6 typeId.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u16)]
        pub enum DmTypeId {
            $( $(#[$doc])* $kind = $value, )*
        }

        impl DmTypeId {
            /// Every allocated kind, in typeId order.
            pub const ALL: &'static [Self] = &[$( Self::$kind, )*];

            /// The kind of a wire typeId; `None` when §6 has not allocated it.
            #[must_use]
            pub fn from_u16(value: u16) -> Option<Self> {
                match value {
                    $( $value => Some(Self::$kind), )*
                    _ => None,
                }
            }

            /// How this kind travels.
            #[must_use]
            pub fn carrier(self) -> Carrier {
                match self {
                    $( Self::$kind => Carrier::$carrier, )*
                }
            }
        }
    };
}

type_ids! {
    /// The PQXDH init, carrying an encrypted first payload (D18).
    PreKeyInit = 0x0001 => PreKeyInit,
    /// A friend request, by `app_call`; the reply is its receipt.
    FriendRequest = 0x0002 => Sealed,
    /// A Strand relay forward (§6 "Relay"), ratchet to the relay friend.
    RelayEnvelope = 0x0003 => Ratchet,
    /// The receipt of entries of the peer's DM stream (plan E2.5, D22):
    /// `delivered` comes only from it.
    DmAck = 0x0004 => Ratchet,
    /// The responder refused a PreKeyInit's prekeys (D18 B5), signed.
    PreKeyRejected = 0x0005 => Sealed,
    /// Allocated E2.3; its body and sends are E2.5's (D24).
    RetryRequest = 0x0006 => Ratchet,
    /// Ask a friend to start a new session (contents defined by E2.4).
    SessionResetRequest = 0x0007 => Sealed,
    /// The first payload of the PreKeyInit that answers a reset request.
    SessionResetAccept = 0x0008 => FirstPayload,
    /// Decline a session reset.
    SessionResetDecline = 0x0009 => Sealed,
    /// A text message, with `reply_to` and `expire_seconds`.
    DmText = 0x0100 => Ratchet,
    /// The first payload of the PreKeyInit that accepts a friend request.
    FriendAccept = 0x0101 => FirstPayload,
    // 0x0102 (FriendReject) is retired: a decline is silent to the
    // requester (LV23 review L2). Never reused.
    /// We removed the peer as a friend.
    Unfriended = 0x0103 => Ratchet,
    /// The receipt of `Unfriended`.
    UnfriendedAck = 0x0104 => Ratchet,
    /// Our profile record key changed.
    ProfileKeyRotated = 0x0105 => Ratchet,
    /// Our friend-presence key (plan E2.5), rotated on unfriend or block.
    FriendsKey = 0x0106 => Ratchet,
    /// The record key of our dm participant record for this conversation,
    /// whose chain holds our stream to the peer (plan E2.5).
    StreamRecord = 0x0107 => Ratchet,
    /// Our identity moved to a new key, over the old session.
    IdentityRotated = 0x0108 => Ratchet,
    /// Edit one of our messages.
    DmEdit = 0x0110 => Ratchet,
    /// Delete one of our messages.
    DmDelete = 0x0111 => Ratchet,
    /// React to a message.
    DmReaction = 0x0112 => Ratchet,
    /// The creator's signed group-DM invite and sending key (plan E2.6).
    GroupDmInvite = 0x0120 => Ratchet,
    /// We joined a group DM: our sending key, to each member.
    GroupDmAccept = 0x0121 => Ratchet,
    /// We declined a group-DM invite.
    GroupDmDecline = 0x0122 => Ratchet,
    /// We left a group DM.
    GroupDmLeave = 0x0123 => Ratchet,
    /// Group-DM content sealed under the sender's key, on its group chain.
    GroupContent = 0x0124 => GroupChain,
    /// The creator grew the roster (plan E7.2 G8): its re-signed invite at
    /// the next epoch, to each member already on it.
    GroupDmRosterGrow = 0x0125 => Ratchet,
    /// The creator renamed the group (plan E7.2 G7): sealed only inside a
    /// `GroupContent` entry's inner, so every member reads it from the
    /// creator's chain; never a frame of its own.
    GroupDmRename = 0x0126 => GroupChain,
    /// A member opened a game lobby in the group, as its host (plan E7.2
    /// G6, SK2-G §5). This and the four lobby kinds after it are sealed
    /// only inside a `GroupContent` entry's inner; never a frame of their
    /// own.
    LobbyOpen = 0x0127 => GroupChain,
    /// A member joined a lobby (G6).
    LobbyJoin = 0x0128 => GroupChain,
    /// A member left a lobby (G6).
    LobbyLeave = 0x0129 => GroupChain,
    /// The host launched its lobby on a server (G6).
    LobbyLaunch = 0x012A => GroupChain,
    /// The host closed its lobby (G6).
    LobbyClose = 0x012B => GroupChain,
    /// A friend offers a relay route.
    RelayOffer = 0x0130 => Ratchet,
    /// The relay friend withdraws its offer.
    RelayWithdraw = 0x0131 => Ratchet,
    /// The `app_call` reply to `RelayOffer`.
    RelayOfferAck = 0x0132 => Ratchet,
    /// Ask a relay for a friend's cached status.
    StatusRequest = 0x0140 => Ratchet,
    /// The cached status.
    StatusResponse = 0x0141 => Ratchet,
    /// The conversation's disappearing-message timer (plan E7.1); a
    /// group's names the group.
    DisappearingTimer = 0x0160 => Ratchet,
    /// An invite to join us on a game server (plan E7.2).
    GameInvite = 0x0170 => Ratchet,
    /// Our bio (plan E7.2), on change and to a newly accepted friend.
    ProfileBio = 0x0171 => Ratchet,
    /// Our game totals (plan E7.2), sent only while we share them.
    GameTotals = 0x0172 => Ratchet,
    /// The creator names a friend a co-creator of a reserved community
    /// (plan E3.2 S7s.3, decision (16)).
    CreatorInvite = 0x0190 => Ratchet,
    /// The co-creator's pseudonym for the anchor, signed by it.
    CreatorAccept = 0x0191 => Ratchet,
    /// The co-creator declines.
    CreatorDecline = 0x0192 => Ratchet,
    /// The creator removed the co-creator or abandoned the creation
    /// (decision (20)).
    CreatorWithdrawn = 0x0193 => Ratchet,
    /// The genesis lists the co-creator: the invite link it joins with
    /// (decision (17)).
    CreatorJoin = 0x0194 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallInvite = 0x0200 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallRinging = 0x0201 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallAccept = 0x0202 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallDecline = 0x0203 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallEnd = 0x0204 => Ratchet,
    /// Call signaling, moved unchanged (obsolete at E4.2).
    CallMediaState = 0x0205 => Ratchet,
    /// An in-call reaction, unchanged.
    CallReaction = 0x0206 => Ratchet,
    /// The group-call invitation.
    GroupCallOffer = 0x0210 => Ratchet,
    /// The group-call invitation's accept.
    GroupCallAccept = 0x0211 => Ratchet,
    /// The group-call invitation's decline.
    GroupCallDecline = 0x0212 => Ratchet,
    /// Obsolete at E4.2.
    GroupCallParticipantJoined = 0x0213 => Ratchet,
    /// Obsolete at E4.2.
    GroupCallParticipantLeft = 0x0214 => Ratchet,
    /// DM video, until E4.3.8 moves it onto the media plane (D12).
    DmVideoFragment = 0x0220 => Ratchet,
    /// Register with a push relay, addressed to its identity.
    RegisterPushRelay = 0x0310 => Sealed,
    /// Revoke a push-relay registration.
    UnregisterPushRelay = 0x0311 => Sealed,
    /// A push relay's wake signal.
    WakeNotify = 0x0312 => Sealed,
    /// The newest of the peer's messages we have read (plan E2.5, r10):
    /// sent only when both sides opted in, never while Invisible.
    ReadUpTo = 0x0400 => Ratchet,
    /// Typing start or stop (expiry in the backend).
    Typing = 0x0401 => Ratchet,
}

impl DmTypeId {
    /// The wire typeId.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// Whether a PreKeyInit may carry this kind as its first payload: the
    /// two accepts, and the content kinds a message to a friend with no
    /// session opens one with (plan E2.3, after an identity rotation), and
    /// the pairwise group control kinds a roster member who is not a friend
    /// opens one with (plan E2.6; never `GroupContent`, a chain entry).
    /// An explicit allowlist (plan audit G1): a removal, its ack, a relay
    /// envelope, an identity rotation, typing and the call kinds never
    /// open a session.
    #[must_use]
    pub fn is_first_payload(self) -> bool {
        matches!(
            self,
            Self::FriendAccept
                | Self::SessionResetAccept
                | Self::DmText
                | Self::DmEdit
                | Self::DmDelete
                | Self::DmReaction
                | Self::GroupDmInvite
                | Self::GroupDmAccept
                | Self::GroupDmDecline
                | Self::GroupDmLeave
        )
    }

    /// Whether this kind is ephemeral: worth nothing once it is late, so it
    /// is never recorded in `unprocessed`, replayed or queued for a retry
    /// (plan audit G10). Typing, and the call and DM-video kinds, whose
    /// state lives only as long as the call.
    #[must_use]
    pub fn is_ephemeral(self) -> bool {
        matches!(
            self,
            Self::Typing
                | Self::CallInvite
                | Self::CallRinging
                | Self::CallAccept
                | Self::CallDecline
                | Self::CallEnd
                | Self::CallMediaState
                | Self::CallReaction
                | Self::GroupCallOffer
                | Self::GroupCallAccept
                | Self::GroupCallDecline
                | Self::GroupCallParticipantJoined
                | Self::GroupCallParticipantLeft
                | Self::DmVideoFragment
        )
    }
}

#[cfg(test)]
#[path = "type_id_tests.rs"]
mod tests;
