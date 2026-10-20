//! `CryptoEvent` samples, covering both `PrekeyPublishKind` variants.

use rekindle_types::subscription_events::{CryptoEvent, PrekeyPublishKind, SubscriptionEvent};

use crate::{channel, community, pk, Sample};

pub const NAMES: &[&str] = &[
    "mekRotated",
    "mekRequested",
    "mekTransferred",
    "adminKeypairGranted",
    "slotKeypairGranted",
    "prekeyRecordPublished",
];

pub const NESTED: &[&str] = &[
    "PrekeyPublishKind.signedRotated",
    "PrekeyPublishKind.oneTimeRefilled",
];

pub fn variant_name(e: &CryptoEvent) -> &'static str {
    match e {
        CryptoEvent::MekRotated { .. } => "mekRotated",
        CryptoEvent::MekRequested { .. } => "mekRequested",
        CryptoEvent::MekTransferred { .. } => "mekTransferred",
        CryptoEvent::AdminKeypairGranted { .. } => "adminKeypairGranted",
        CryptoEvent::SlotKeypairGranted { .. } => "slotKeypairGranted",
        CryptoEvent::PrekeyRecordPublished { .. } => "prekeyRecordPublished",
    }
}

fn kind_name(kind: PrekeyPublishKind) -> &'static str {
    match kind {
        PrekeyPublishKind::SignedRotated => "PrekeyPublishKind.signedRotated",
        PrekeyPublishKind::OneTimeRefilled => "PrekeyPublishKind.oneTimeRefilled",
    }
}

pub fn nested_names(e: &CryptoEvent) -> Vec<&'static str> {
    match e {
        CryptoEvent::PrekeyRecordPublished { kind, .. } => vec![kind_name(*kind)],
        CryptoEvent::MekRotated { .. }
        | CryptoEvent::MekRequested { .. }
        | CryptoEvent::MekTransferred { .. }
        | CryptoEvent::AdminKeypairGranted { .. }
        | CryptoEvent::SlotKeypairGranted { .. } => vec![],
    }
}

fn ev(name: &str, e: CryptoEvent) -> Sample {
    (format!("crypto.{name}"), SubscriptionEvent::Crypto(e))
}

pub fn samples() -> Vec<Sample> {
    vec![
        ev(
            "mekRotated",
            CryptoEvent::MekRotated {
                community: community(),
                channel: Some(channel()),
                generation: 5,
                rotator_pseudonym: Some(pk('a')),
            },
        ),
        ev(
            "mekRequested",
            CryptoEvent::MekRequested {
                community: community(),
                channel: channel(),
                needed_generation: 4,
                requester_pseudonym: pk('e'),
            },
        ),
        ev(
            "mekTransferred",
            CryptoEvent::MekTransferred {
                community: community(),
                channel: Some(channel()),
                generation: 5,
                sender_pseudonym: pk('a'),
            },
        ),
        ev(
            "adminKeypairGranted",
            CryptoEvent::AdminKeypairGranted {
                community: community(),
            },
        ),
        ev(
            "slotKeypairGranted",
            CryptoEvent::SlotKeypairGranted {
                community: community(),
                slot_index: 17,
                segment_index: 1,
            },
        ),
        ev(
            "prekeyRecordPublished.signedRotated",
            CryptoEvent::PrekeyRecordPublished {
                kind: PrekeyPublishKind::SignedRotated,
                subkey: 0,
            },
        ),
        ev(
            "prekeyRecordPublished.oneTimeRefilled",
            CryptoEvent::PrekeyRecordPublished {
                kind: PrekeyPublishKind::OneTimeRefilled,
                subkey: 37,
            },
        ),
    ]
}
