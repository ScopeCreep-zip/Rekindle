//! `VoiceEvent` samples, covering both `VoiceScope`s.

use rekindle_types::subscription_events::{
    LinkMeasurement, SubscriptionEvent, VoiceEvent, VoiceParticipant, VoiceScope,
};

use crate::{community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "joined",
    "left",
    "modeChanged",
    "muteChanged",
    "deafenChanged",
    "rosterUpdated",
    "joinHandshake",
    "peerConfirmed",
    "mediaReady",
    "stageUpdated",
    "speakRequested",
    "soundboardPlayed",
    "speakResponded",
    "localJoined",
    "speakingChanged",
    "deviceChanged",
    "packetsDropped",
    "connectionQuality",
];

pub const NESTED: &[&str] = &["VoiceScope.community", "VoiceScope.dm"];

pub fn variant_name(e: &VoiceEvent) -> &'static str {
    match e {
        VoiceEvent::Joined { .. } => "joined",
        VoiceEvent::Left { .. } => "left",
        VoiceEvent::ModeChanged { .. } => "modeChanged",
        VoiceEvent::MuteChanged { .. } => "muteChanged",
        VoiceEvent::DeafenChanged { .. } => "deafenChanged",
        VoiceEvent::RosterUpdated { .. } => "rosterUpdated",
        VoiceEvent::JoinHandshake { .. } => "joinHandshake",
        VoiceEvent::PeerConfirmed { .. } => "peerConfirmed",
        VoiceEvent::MediaReady { .. } => "mediaReady",
        VoiceEvent::StageUpdated { .. } => "stageUpdated",
        VoiceEvent::SpeakRequested { .. } => "speakRequested",
        VoiceEvent::SoundboardPlayed { .. } => "soundboardPlayed",
        VoiceEvent::SpeakResponded { .. } => "speakResponded",
        VoiceEvent::LocalJoined { .. } => "localJoined",
        VoiceEvent::SpeakingChanged { .. } => "speakingChanged",
        VoiceEvent::DeviceChanged { .. } => "deviceChanged",
        VoiceEvent::PacketsDropped { .. } => "packetsDropped",
        VoiceEvent::ConnectionQuality { .. } => "connectionQuality",
    }
}

fn scope_name(scope: &VoiceScope) -> &'static str {
    match scope {
        VoiceScope::Community { .. } => "VoiceScope.community",
        VoiceScope::Dm { .. } => "VoiceScope.dm",
    }
}

pub fn nested_names(e: &VoiceEvent) -> Vec<&'static str> {
    e.scope().into_iter().map(scope_name).collect()
}

fn ev(name: &str, e: VoiceEvent) -> Sample {
    (format!("voice.{name}"), SubscriptionEvent::Voice(e))
}

fn room() -> VoiceScope {
    VoiceScope::Community {
        community: community(),
        channel: id("vc_", 'c'),
    }
}

fn dm() -> VoiceScope {
    VoiceScope::Dm { peer_key: pk('b') }
}

/// Exactly representable floats, so the golden text is stable.
fn link() -> LinkMeasurement {
    LinkMeasurement {
        loss_q8: 3,
        discard_q8: 1,
        jitter_ms: 12,
        rtt_ms: Some(80),
        r_factor: 88,
        mos_lq: 4.25,
        mos_cq: 4.0,
        bitrate_bps: 32_000,
        one_way_p50_ms: Some(40),
        one_way_p95_ms: Some(65),
        mouth_to_ear_ms: Some(120),
        outside_g114: false,
    }
}

pub fn samples() -> Vec<Sample> {
    let mut all = roster();
    all.extend(moderation());
    all.extend(local_session());
    all
}

fn joined(suffix: &str, scope: VoiceScope) -> Sample {
    ev(
        &format!("joined.{suffix}"),
        VoiceEvent::Joined {
            scope,
            pseudonym: pk('e'),
            display_name: Some(text("Eve")),
            route_blob: Some(vec![1, 2, 3, 4]),
        },
    )
}

fn roster() -> Vec<Sample> {
    vec![
        joined("community", room()),
        joined("dm", dm()),
        ev(
            "left",
            VoiceEvent::Left {
                scope: room(),
                pseudonym: pk('e'),
            },
        ),
        ev(
            "modeChanged",
            VoiceEvent::ModeChanged {
                scope: room(),
                mode: text("sfu"),
                host_pseudonym: Some(pk('a')),
            },
        ),
        ev(
            "rosterUpdated",
            VoiceEvent::RosterUpdated {
                scope: room(),
                participants: vec![VoiceParticipant {
                    pseudonym_key: pk('e'),
                    display_name: Some(text("Eve")),
                }],
            },
        ),
        ev(
            "joinHandshake",
            VoiceEvent::JoinHandshake {
                scope: room(),
                state: text("offerSent"),
                peer: Some(pk('a')),
                display_name: Some(text("Alice")),
            },
        ),
        ev(
            "peerConfirmed",
            VoiceEvent::PeerConfirmed {
                scope: room(),
                pseudonym: pk('a'),
            },
        ),
        ev(
            "mediaReady",
            VoiceEvent::MediaReady {
                scope: room(),
                ready: true,
                reason: text("mediaRouteAvailable"),
            },
        ),
    ]
}

fn moderation() -> Vec<Sample> {
    vec![
        ev(
            "muteChanged",
            VoiceEvent::MuteChanged {
                scope: room(),
                target_pseudonym: pk('e'),
                muted: true,
            },
        ),
        ev(
            "deafenChanged",
            VoiceEvent::DeafenChanged {
                scope: room(),
                target_pseudonym: pk('e'),
                deafened: true,
            },
        ),
        ev(
            "stageUpdated",
            VoiceEvent::StageUpdated {
                scope: room(),
                topic: Some(text("Patch notes")),
                speakers: vec![pk('a')],
                moderator_pseudonym: pk('a'),
            },
        ),
        ev(
            "speakRequested",
            VoiceEvent::SpeakRequested {
                scope: room(),
                requester_pseudonym: pk('e'),
            },
        ),
        ev(
            "soundboardPlayed",
            VoiceEvent::SoundboardPlayed {
                scope: room(),
                expression_id: id("exp_", '2'),
                actor_pseudonym: pk('e'),
            },
        ),
        ev(
            "speakResponded",
            VoiceEvent::SpeakResponded {
                scope: room(),
                requester_pseudonym: pk('e'),
                granted: true,
                moderator_pseudonym: pk('a'),
            },
        ),
    ]
}

fn connection_quality(suffix: &str, scope: VoiceScope) -> Sample {
    ev(
        &format!("connectionQuality.{suffix}"),
        VoiceEvent::ConnectionQuality {
            scope,
            quality: text("good"),
            rx_overflow_drops: 1,
            rx_late_drops: 2,
            rx_key_drops: 0,
            ingress_drops: 3,
            link: Some(link()),
        },
    )
}

fn local_session() -> Vec<Sample> {
    vec![
        ev(
            "localJoined.community",
            VoiceEvent::LocalJoined { scope: room() },
        ),
        ev("localJoined.dm", VoiceEvent::LocalJoined { scope: dm() }),
        ev(
            "speakingChanged",
            VoiceEvent::SpeakingChanged {
                scope: dm(),
                pseudonym: pk('b'),
                speaking: true,
            },
        ),
        ev(
            "deviceChanged",
            VoiceEvent::DeviceChanged {
                device_type: text("input"),
                device_name: text("USB Headset"),
                reason: text("unplugged"),
            },
        ),
        ev(
            "packetsDropped",
            VoiceEvent::PacketsDropped {
                scope: room(),
                reason: text("jitterOverflow"),
                count: 9,
            },
        ),
        connection_quality("community", room()),
        connection_quality("dm", dm()),
    ]
}
