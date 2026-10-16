//! `SocialEvent` samples, covering every `EventLocation` across the
//! scheduled-event variants.

use rekindle_types::event::{
    DayOfWeek, EventInfo, EventLocation, EventRsvp, RecurrenceFrequency, RecurrenceRule,
};
use rekindle_types::game_server::GameServerInfo;
use rekindle_types::id::ChannelId;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};
use rekindle_types::thread::ThreadInfo;

use crate::{channel, community, id, pk, text, Sample};

pub const NAMES: &[&str] = &[
    "reactionAdded",
    "reactionRemoved",
    "messagePinned",
    "messageUnpinned",
    "threadCreated",
    "threadMessagePosted",
    "threadArchiveChanged",
    "eventCreated",
    "eventUpdated",
    "eventDeleted",
    "eventRsvpChanged",
    "eventReminder",
    "gameServerAdded",
    "gameServerRemoved",
];

pub const NESTED: &[&str] = &[
    "EventLocation.voiceChannel",
    "EventLocation.stageChannel",
    "EventLocation.external",
    "EventLocation.inGame",
];

pub fn variant_name(e: &SocialEvent) -> &'static str {
    match e {
        SocialEvent::ReactionAdded { .. } => "reactionAdded",
        SocialEvent::ReactionRemoved { .. } => "reactionRemoved",
        SocialEvent::MessagePinned { .. } => "messagePinned",
        SocialEvent::MessageUnpinned { .. } => "messageUnpinned",
        SocialEvent::ThreadCreated { .. } => "threadCreated",
        SocialEvent::ThreadMessagePosted { .. } => "threadMessagePosted",
        SocialEvent::ThreadArchiveChanged { .. } => "threadArchiveChanged",
        SocialEvent::EventCreated { .. } => "eventCreated",
        SocialEvent::EventUpdated { .. } => "eventUpdated",
        SocialEvent::EventDeleted { .. } => "eventDeleted",
        SocialEvent::EventRsvpChanged { .. } => "eventRsvpChanged",
        SocialEvent::EventReminder { .. } => "eventReminder",
        SocialEvent::GameServerAdded { .. } => "gameServerAdded",
        SocialEvent::GameServerRemoved { .. } => "gameServerRemoved",
    }
}

fn location_name(location: &EventLocation) -> &'static str {
    match location {
        EventLocation::VoiceChannel(_) => "EventLocation.voiceChannel",
        EventLocation::StageChannel(_) => "EventLocation.stageChannel",
        EventLocation::External(_) => "EventLocation.external",
        EventLocation::InGame { .. } => "EventLocation.inGame",
    }
}

pub fn nested_names(e: &SocialEvent) -> Vec<&'static str> {
    match e {
        SocialEvent::EventCreated { event, .. } | SocialEvent::EventUpdated { event, .. } => {
            event.location.iter().map(location_name).collect()
        }
        SocialEvent::ReactionAdded { .. }
        | SocialEvent::ReactionRemoved { .. }
        | SocialEvent::MessagePinned { .. }
        | SocialEvent::MessageUnpinned { .. }
        | SocialEvent::ThreadCreated { .. }
        | SocialEvent::ThreadMessagePosted { .. }
        | SocialEvent::ThreadArchiveChanged { .. }
        | SocialEvent::EventDeleted { .. }
        | SocialEvent::EventRsvpChanged { .. }
        | SocialEvent::EventReminder { .. }
        | SocialEvent::GameServerAdded { .. }
        | SocialEvent::GameServerRemoved { .. } => vec![],
    }
}

fn ev(name: &str, e: SocialEvent) -> Sample {
    (format!("social.{name}"), SubscriptionEvent::Social(e))
}

fn event_id() -> String {
    id("evt_", '5')
}

fn thread_id() -> String {
    id("thr_", '6')
}

/// A fully populated scheduled event at `location`.
fn event_info(location: EventLocation) -> Box<EventInfo> {
    Box::new(EventInfo {
        id: event_id(),
        title: text("Raid night"),
        description: text("Bring potions"),
        creator_pseudonym: pk('a'),
        start_time: 1_760_100_000_000,
        end_time: Some(1_760_107_200_000),
        channel_id: Some(channel()),
        max_attendees: Some(40),
        created_at: 1_760_000_000_000,
        status: text("scheduled"),
        rsvps: vec![EventRsvp {
            pseudonym_key: pk('e'),
            status: text("going"),
        }],
        cover_image_ref: Some(format!("blake3:{}", "8".repeat(64))),
        recurrence: Some(RecurrenceRule {
            frequency: RecurrenceFrequency::Weekly,
            interval: 1,
            days_of_week: Some(vec![DayOfWeek::Friday]),
            until: Some(1_770_000_000_000),
            count: Some(10),
        }),
        location: Some(location),
    })
}

pub fn samples() -> Vec<Sample> {
    let mut all = messages_and_threads();
    all.extend(scheduled_events());
    all.extend(game_servers());
    all
}

fn messages_and_threads() -> Vec<Sample> {
    vec![
        ev(
            "reactionAdded",
            SocialEvent::ReactionAdded {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                emoji: text("🔥"),
                reactor_pseudonym: pk('e'),
            },
        ),
        ev(
            "reactionRemoved",
            SocialEvent::ReactionRemoved {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                emoji: text("🔥"),
                reactor_pseudonym: pk('e'),
            },
        ),
        ev(
            "messagePinned",
            SocialEvent::MessagePinned {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
                pinned_by: pk('a'),
            },
        ),
        ev(
            "messageUnpinned",
            SocialEvent::MessageUnpinned {
                community: community(),
                channel: channel(),
                message_id: id("msg_", '1'),
            },
        ),
        ev(
            "threadCreated",
            SocialEvent::ThreadCreated {
                community: community(),
                thread: Box::new(ThreadInfo {
                    id: thread_id(),
                    channel_id: channel(),
                    name: text("Strategy"),
                    starter_message_id: id("msg_", '1'),
                    creator_pseudonym: pk('a'),
                    forum_tag: Some(text("guides")),
                    created_at: 1_760_000_000_000,
                    archived: false,
                    auto_archive_seconds: 86_400,
                    last_message_at: 1_760_000_500_000,
                    message_count: 4,
                }),
            },
        ),
        ev(
            "threadMessagePosted",
            SocialEvent::ThreadMessagePosted {
                community: community(),
                thread_id: thread_id(),
                message_id: id("msg_", '7'),
                sender_pseudonym: pk('e'),
                timestamp: 1_760_000_500_000,
                body: Some(text("flank left")),
                reply_to_id: Some(id("msg_", '1')),
            },
        ),
        ev(
            "threadArchiveChanged",
            SocialEvent::ThreadArchiveChanged {
                community: community(),
                thread_id: thread_id(),
                archived: true,
            },
        ),
    ]
}

fn scheduled_events() -> Vec<Sample> {
    vec![
        ev(
            "eventCreated.voiceChannel",
            SocialEvent::EventCreated {
                community: community(),
                event: event_info(EventLocation::VoiceChannel(ChannelId([0xAB; 16]))),
            },
        ),
        ev(
            "eventCreated.external",
            SocialEvent::EventCreated {
                community: community(),
                event: event_info(EventLocation::External(text("https://example.org/lan"))),
            },
        ),
        ev(
            "eventUpdated.stageChannel",
            SocialEvent::EventUpdated {
                community: community(),
                event: event_info(EventLocation::StageChannel(ChannelId([0xCD; 16]))),
            },
        ),
        ev(
            "eventUpdated.inGame",
            SocialEvent::EventUpdated {
                community: community(),
                event: event_info(EventLocation::InGame {
                    game_id: 4_101,
                    server_address: Some(text("10.0.0.1:16567")),
                }),
            },
        ),
        ev(
            "eventDeleted",
            SocialEvent::EventDeleted {
                community: community(),
                event_id: event_id(),
            },
        ),
        ev(
            "eventRsvpChanged",
            SocialEvent::EventRsvpChanged {
                community: community(),
                event_id: event_id(),
                pseudonym: pk('e'),
                rsvp_status: text("interested"),
            },
        ),
        ev(
            "eventReminder",
            SocialEvent::EventReminder {
                community: community(),
                event_id: event_id(),
                title: text("Raid night"),
                minutes_until_start: 15,
            },
        ),
    ]
}

fn game_servers() -> Vec<Sample> {
    vec![
        ev(
            "gameServerAdded",
            SocialEvent::GameServerAdded {
                community: community(),
                server: Box::new(GameServerInfo {
                    id: id("gs_", '3'),
                    game_id: text("4101"),
                    label: text("Friday CTF"),
                    address: text("10.0.0.1:27015"),
                    added_by: pk('a'),
                    created_at: 1_760_000_000_000,
                }),
            },
        ),
        ev(
            "gameServerRemoved",
            SocialEvent::GameServerRemoved {
                community: community(),
                server_id: id("gs_", '3'),
            },
        ),
    ]
}
