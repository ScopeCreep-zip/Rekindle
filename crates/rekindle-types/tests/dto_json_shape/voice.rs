use rekindle_types::dto::voice::{VoiceControls, VoiceJoined, VoiceLeft};
use serde_json::Value;

use super::to;

pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    let community = "community".to_owned();
    let channel = "04".repeat(16);
    add(
        "voice.joined",
        to(VoiceJoined {
            joined: true,
            community: community.clone(),
            channel: channel.clone(),
            muted: false,
            deafened: false,
        }),
    );
    add(
        "voice.left",
        to(VoiceLeft {
            left: true,
            community,
            channel,
        }),
    );
    add(
        "voice.controls",
        to(VoiceControls {
            muted: true,
            deafened: false,
        }),
    );
}
