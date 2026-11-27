//! The Host's words for each community capability a member lacks (plan
//! E3.2 S9): the reason a community window shows on a disabled control,
//! from the real cause `rekindle_governance::permissions::denied_capabilities`
//! names (a missing permission, a running timeout, a creator seat), never a
//! generic "not allowed". The window renders the text as given; it reads no
//! permission bits for its controls.

use chrono::{DateTime, Local, TimeZone as _};
use rekindle_governance::permissions::{denied_capabilities, denied_in_channel, CapabilityDenial};
use rekindle_governance::state::GovernanceState;
use rekindle_governance::validate::target::TargetRefusal;
use rekindle_types::dto::community_capability::{
    ChannelDeniedCapabilities, CommunityCapability, DeniedCapabilities,
};
use rekindle_types::id::{ChannelId, PseudonymKey};

/// A member row's actions before we hold a pseudonym here.
pub(super) const JOIN_UNFINISHED: &str = "You haven't finished joining this community yet.";
/// Every control before the community's first merge.
pub(super) const NOT_LOADED: &str = "This community's rules have not loaded yet.";
/// Our own row's actions.
pub(super) const YOURSELF: &str = "You can't moderate yourself.";

/// The Host's words for a moderation action's refusal against one member.
pub(super) fn moderation_words(refusal: TargetRefusal) -> String {
    match refusal {
        TargetRefusal::SeatedCreator => {
            "You can't moderate a creator of this community.".to_owned()
        }
        TargetRefusal::WriterBanned => "You're banned from this community.".to_owned(),
        TargetRefusal::Missing { permission } => format!("You need the {permission} permission."),
        TargetRefusal::Rank => {
            "You can't moderate someone at or above your highest role.".to_owned()
        }
        TargetRefusal::NoRoles => "This community has no roles yet.".to_owned(),
        TargetRefusal::NoAssignableRoles => "There are no roles you can assign.".to_owned(),
    }
}

/// `member`'s denied capabilities in `state` at `now_secs`, worded.
pub(super) fn denied_of(
    member: &PseudonymKey,
    state: &GovernanceState,
    now_secs: u64,
) -> DeniedCapabilities {
    denied_capabilities(member, state, now_secs)
        .into_iter()
        .map(|(cap, denial)| (cap, words(cap, denial, now_secs)))
        .collect()
}

/// `member`'s denied channel capabilities in `channel` of `state` at
/// `now_secs`, worded.
pub(super) fn channel_denied_of(
    member: &PseudonymKey,
    channel: &ChannelId,
    state: &GovernanceState,
    now_secs: u64,
) -> ChannelDeniedCapabilities {
    denied_in_channel(member, channel, state, now_secs)
        .into_iter()
        .map(|(cap, denial)| (cap, denial_words(denial, now_secs)))
        .collect()
}

fn words(cap: CommunityCapability, denial: CapabilityDenial, now_secs: u64) -> String {
    match (cap, denial) {
        (CommunityCapability::OfferSuccessor, CapabilityDenial::NotCreator) => {
            "Only a creator of this community can offer a successor community.".to_owned()
        }
        (_, denial) => denial_words(denial, now_secs),
    }
}

fn denial_words(denial: CapabilityDenial, now_secs: u64) -> String {
    match denial {
        CapabilityDenial::Missing { permission } => {
            format!("You need the {permission} permission.")
        }
        CapabilityDenial::TimedOut { until_secs } => {
            format!("You're timed out until {}.", clock(until_secs, now_secs))
        }
        CapabilityDenial::NotCreator => "Only a creator of this community can do this.".to_owned(),
        CapabilityDenial::ChannelDenied { permission } => {
            format!("{permission} is turned off for you in this channel.")
        }
    }
}

/// `until_secs` on the local clock: `HH:MM` on the same local day as
/// `now_secs`, else with its date (`Dec 3, HH:MM`).
pub(super) fn clock(until_secs: u64, now_secs: u64) -> String {
    let local = |secs: u64| -> Option<DateTime<Local>> {
        Local.timestamp_opt(i64::try_from(secs).ok()?, 0).single()
    };
    match (local(until_secs), local(now_secs)) {
        (Some(until), Some(now)) if until.date_naive() == now.date_naive() => {
            until.format("%H:%M").to_string()
        }
        (Some(until), _) => until.format("%b %-d, %H:%M").to_string(),
        // Past what the clock can show: the instant in seconds, still exact.
        (None, _) => format!("unix time {until_secs}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_is_worded_as_itself() {
        assert_eq!(
            words(
                CommunityCapability::ManageChannels,
                CapabilityDenial::Missing {
                    permission: "Manage Channels"
                },
                0
            ),
            "You need the Manage Channels permission."
        );
        assert_eq!(
            words(
                CommunityCapability::BanMembers,
                CapabilityDenial::TimedOut { until_secs: 7_200 },
                3_600
            ),
            format!("You're timed out until {}.", clock(7_200, 3_600))
        );
        assert_eq!(
            words(
                CommunityCapability::OfferSuccessor,
                CapabilityDenial::NotCreator,
                0
            ),
            "Only a creator of this community can offer a successor community."
        );
    }

    #[test]
    fn a_channel_overwrite_is_named_as_the_cause() {
        assert_eq!(
            denial_words(
                CapabilityDenial::ChannelDenied {
                    permission: "Manage Messages"
                },
                0
            ),
            "Manage Messages is turned off for you in this channel."
        );
    }

    #[test]
    fn the_clock_names_the_date_only_off_today() {
        let now = 1_800_000_000;
        assert_eq!(clock(now, now).len(), "HH:MM".len());
        assert!(clock(now + 3 * 86_400, now).contains(", "));
    }
}
