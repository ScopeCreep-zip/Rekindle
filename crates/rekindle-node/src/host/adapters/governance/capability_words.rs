//! CAPR's Host wording for channel capability denials.

use rekindle_governance::permissions::{denied_in_channel, CapabilityDenial};
use rekindle_governance::state::GovernanceState;
use rekindle_types::dto::community_capability::{ChannelCapability, ChannelDeniedCapabilities};
use rekindle_types::id::{ChannelId, PseudonymKey};

/// The merged rules must be present to decide these controls.
pub(crate) const NOT_LOADED: &str = "This community's rules have not loaded yet.";

pub(crate) fn joined_pseudonym(hex: &str) -> Option<PseudonymKey> {
    hex::decode(hex).ok()?.try_into().ok().map(PseudonymKey)
}

pub(super) fn channel_controls_loading() -> ChannelDeniedCapabilities {
    ChannelCapability::ALL
        .into_iter()
        .map(|capability| (capability, NOT_LOADED.to_owned()))
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

pub(crate) fn denial_words(denial: CapabilityDenial, now_secs: u64) -> String {
    match denial {
        CapabilityDenial::Missing { permission } => {
            format!("You need the {permission} permission.")
        }
        CapabilityDenial::TimedOut { until_secs } => {
            let remaining = until_secs.saturating_sub(now_secs);
            let seconds = u32::try_from(remaining).unwrap_or(u32::MAX);
            let duration = rekindle_types::disappearing::duration_text(seconds);
            format!("Your timeout ends in {duration}.")
        }
        CapabilityDenial::ChannelDenied { permission } => {
            format!("{permission} is turned off for you in this channel.")
        }
    }
}
