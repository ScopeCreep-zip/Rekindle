//! Community departure reasons and their subscription events.

/// Why our hold of a community ends, which names the one event every
/// frontend hears when its store entry goes (one event per cause).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldEnd {
    /// Our leave: `Forgotten`.
    Left,
    /// The join call's own refusal, before the community was announced
    /// (`SelfJoined`): the call's error tells the caller; `Forgotten` is a
    /// no-op on every frontend that never listed it.
    JoinRefusedAtOnce,
    /// The inviter refused a join after it returned (plan E3.1/JN1): only
    /// `JoinWithdrawn`, so a community window holding it keeps it and
    /// shows the withdrawal in place (plan JW ruling).
    JoinWithdrawn(rekindle_types::join_refusal::JoinRefusal),
    /// Our Forget of a community we were removed from: `Forgotten`.
    ForgetAfterRemoval,
}

impl HoldEnd {
    /// The event every frontend hears for this end of `community`.
    pub(super) fn event(
        self,
        community: &str,
    ) -> rekindle_types::subscription_events::SubscriptionEvent {
        use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};
        let community = community.to_owned();
        SubscriptionEvent::Membership(match self {
            Self::Left | Self::JoinRefusedAtOnce | Self::ForgetAfterRemoval => {
                MembershipEvent::Forgotten { community }
            }
            Self::JoinWithdrawn(reason) => MembershipEvent::JoinWithdrawn {
                community,
                reason,
                reason_text: reason.withdrawn_text(),
            },
        })
    }
}
