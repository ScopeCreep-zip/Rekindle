//! `TypingEvent` samples, each variant in both `TypingContext`s.

use rekindle_types::subscription_events::{SubscriptionEvent, TypingContext, TypingEvent};

use crate::{channel, community, pk, Sample};

pub const NAMES: &[&str] = &["started", "stopped"];

pub const NESTED: &[&str] = &["TypingContext.channel", "TypingContext.dm"];

pub fn variant_name(e: &TypingEvent) -> &'static str {
    match e {
        TypingEvent::Started { .. } => "started",
        TypingEvent::Stopped { .. } => "stopped",
    }
}

fn context_name(context: &TypingContext) -> &'static str {
    match context {
        TypingContext::Channel { .. } => "TypingContext.channel",
        TypingContext::Dm { .. } => "TypingContext.dm",
    }
}

pub fn nested_names(e: &TypingEvent) -> Vec<&'static str> {
    match e {
        TypingEvent::Started { context, .. } | TypingEvent::Stopped { context, .. } => {
            vec![context_name(context)]
        }
    }
}

fn ev(name: &str, e: TypingEvent) -> Sample {
    (format!("typing.{name}"), SubscriptionEvent::Typing(e))
}

fn in_channel() -> TypingContext {
    TypingContext::Channel {
        community: community(),
        channel: channel(),
    }
}

fn in_dm() -> TypingContext {
    TypingContext::Dm { peer_key: pk('b') }
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "started.channel",
            TypingEvent::Started {
                context: in_channel(),
                who: pk('a'),
            },
        ),
        ev(
            "started.dm",
            TypingEvent::Started {
                context: in_dm(),
                who: pk('b'),
            },
        ),
        ev(
            "stopped.channel",
            TypingEvent::Stopped {
                context: in_channel(),
                who: pk('a'),
            },
        ),
        ev(
            "stopped.dm",
            TypingEvent::Stopped {
                context: in_dm(),
                who: pk('b'),
            },
        ),
    ]
}
