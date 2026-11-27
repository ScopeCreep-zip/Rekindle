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
