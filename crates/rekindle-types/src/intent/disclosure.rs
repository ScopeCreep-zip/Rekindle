//! Who a disclosure reaches, why it cannot be taken back, and the
//! acknowledgement a TrustChanging review asks for (ADR 0019: the review
//! states the salient trust fact and its reversibility; the Host owns that
//! content, every frontend renders it).

use serde::{Deserialize, Serialize};

use super::{Escalation, Fact};

/// The third party our IP address reaches ([`Fact::IpAddressLeaves`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Recipient {
    /// The web site a link opens (`LinkOpen`), by the host the browser asks
    /// for.
    #[serde(rename_all = "camelCase")]
    Site { host: String },
    /// The game server a peer supplied (`JoinGameServer`, lead decision
    /// (50)): the game's name and the server's `host:port`.
    #[serde(rename_all = "camelCase")]
    GameServer { game: String, address: String },
}

impl Recipient {
    /// The kind of party, for the wording that names it without its address.
    #[must_use]
    pub const fn kind(&self) -> RecipientKind {
        match self {
            Self::Site { .. } => RecipientKind::Site,
            Self::GameServer { .. } => RecipientKind::GameServer,
        }
    }
}

/// A [`Recipient`]'s kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecipientKind {
    Site,
    GameServer,
}

/// Why an action can never be taken back
/// ([`super::Reversibility::NeverBecause`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// What reached a third party (our IP address) cannot be recalled
    /// once it has seen it.
    #[serde(rename_all = "camelCase")]
    DisclosureCannotBeRecalled { to: RecipientKind },
}

/// The Host's own statement a TrustChanging review's acknowledgement
/// checkbox carries: the salient trust fact in the first person, never the
/// wording of a fact line. The escalation that raised the class leads (a
/// verified key that changed, plan E2.3), then the facts in order.
#[must_use]
pub fn acknowledgement(facts: &[Fact], escalations: &[Escalation]) -> &'static str {
    let from_escalation = escalations.iter().find_map(|e| match e {
        Escalation::VerifiedKeyChanged { .. } => {
            Some("I understand their key changed since I verified it.")
        }
        Escalation::Attachment(_) => Some("I understand this file leaves Rekindle."),
        Escalation::FirstMediaInCall => Some("I understand my camera or screen goes to this call."),
        Escalation::VisibleBeforeUnlock => {
            Some("I understand anyone at this screen can see this skin before I unlock.")
        }
        _ => None,
    });
    let from_fact = || {
        facts.iter().find_map(|f| match f {
            Fact::IpAddressLeaves { to } => Some(match to.kind() {
                RecipientKind::GameServer => "I understand this server will see my IP address.",
                RecipientKind::Site => "I understand this site will see my IP address.",
            }),
            Fact::PresenceVisible => Some("I understand they can see when I am online."),
            _ => None,
        })
    };
    from_escalation
        .or_else(from_fact)
        .unwrap_or("I understand what this changes.")
}
