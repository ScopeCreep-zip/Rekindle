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
    /// Decline a friend request.
    FriendReject = 0x0102 => Sealed,
    /// We removed the peer as a friend.
    Unfriended = 0x0103 => Ratchet,
    /// The receipt of `Unfriended`.
    UnfriendedAck = 0x0104 => Ratchet,
    /// Our profile record key changed.
    ProfileKeyRotated = 0x0105 => Ratchet,
    /// Our identity moved to a new key, over the old session.
    IdentityRotated = 0x0108 => Ratchet,
    /// Edit one of our messages.
    DmEdit = 0x0110 => Ratchet,
    /// Delete one of our messages.
    DmDelete = 0x0111 => Ratchet,
    /// React to a message.
    DmReaction = 0x0112 => Ratchet,
    /// Allocated E2.3; body E2.6.
    GroupDmInvite = 0x0120 => Ratchet,
    /// Allocated E2.3; body E2.6.
    GroupDmAccept = 0x0121 => Ratchet,
    /// Allocated E2.3; body E2.6.
    GroupDmDecline = 0x0122 => Ratchet,
    /// Allocated E2.3; body E2.6.
    GroupDmLeave = 0x0123 => Ratchet,
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
    /// Typing start or stop (expiry in the backend).
    Typing = 0x0401 => Ratchet,
}

impl DmTypeId {
    /// The wire typeId.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// Whether a PreKeyInit may carry this kind as its first payload:
    /// the accept kinds, and any ratchet kind when a session is
    /// (re)initiated with it.
    #[must_use]
    pub fn is_first_payload(self) -> bool {
        matches!(self.carrier(), Carrier::FirstPayload | Carrier::Ratchet)
    }
}

#[cfg(test)]
mod tests {
    use super::{Carrier, DmTypeId};

    #[test]
    fn every_kind_round_trips_and_ids_are_ordered_and_unique() {
        for kind in DmTypeId::ALL {
            assert_eq!(DmTypeId::from_u16(kind.as_u16()), Some(*kind));
        }
        assert!(DmTypeId::ALL
            .windows(2)
            .all(|w| w[0].as_u16() < w[1].as_u16()));
    }

    /// Ids §6 allocates to later steps, or never, are not kinds yet.
    #[test]
    fn unallocated_ids_are_not_kinds() {
        for id in [
            0x0000, 0x0004, 0x0005, 0x0106, 0x0107, 0x0124, 0x0150, 0x0207, 0x0230, 0x0300, 0x0400,
            0xFFFF,
        ] {
            assert_eq!(DmTypeId::from_u16(id), None, "{id:#06x}");
        }
    }

    /// §6's Sealing column.
    #[test]
    fn carriers_follow_the_ledger() {
        let sealed: Vec<u16> = DmTypeId::ALL
            .iter()
            .filter(|k| k.carrier() == Carrier::Sealed)
            .map(|k| k.as_u16())
            .collect();
        assert_eq!(
            sealed,
            [0x0002, 0x0007, 0x0009, 0x0102, 0x0310, 0x0311, 0x0312]
        );
        assert_eq!(DmTypeId::PreKeyInit.carrier(), Carrier::PreKeyInit);
        assert_eq!(DmTypeId::FriendAccept.carrier(), Carrier::FirstPayload);
        assert_eq!(
            DmTypeId::SessionResetAccept.carrier(),
            Carrier::FirstPayload
        );
        assert!(DmTypeId::FriendAccept.is_first_payload());
        assert!(DmTypeId::DmText.is_first_payload());
        assert!(!DmTypeId::PreKeyInit.is_first_payload());
        assert!(!DmTypeId::FriendRequest.is_first_payload());
    }
}
