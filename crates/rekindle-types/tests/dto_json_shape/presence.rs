//! Golden samples for the community presence policy and field edits.

use rekindle_types::presence::{
    LastSeenPrecision, PresencePolicyField, PresenceSharingPolicy, ShareScope,
};

use serde_json::Value;

use super::to;

pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "presenceSharingPolicy",
        to(PresenceSharingPolicy::default()),
    );
    for (name, field) in [
        ("shareOnline", PresencePolicyField::ShareOnline(false)),
        (
            "shareLocation",
            PresencePolicyField::ShareLocation(ShareScope::Members),
        ),
        (
            "shareActivity",
            PresencePolicyField::ShareActivity(ShareScope::None),
        ),
        (
            "lastSeen",
            PresencePolicyField::LastSeen(LastSeenPrecision::Coarse),
        ),
    ] {
        add(&format!("presencePolicyField.{name}"), to(field));
    }
}
