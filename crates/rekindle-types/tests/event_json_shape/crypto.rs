//! `CryptoEvent` samples, covering both `PqBundleKind` variants.

use rekindle_types::subscription_events::{CryptoEvent, PqBundleKind, SubscriptionEvent};

use crate::{channel, community, pk, Sample};

pub const NAMES: &[&str] = &[
    "mekRotated",
    "mekRequested",
    "mekTransferred",
    "adminKeypairGranted",
    "slotKeypairGranted",
    "pqBundlePublished",
];

pub const NESTED: &[&str] = &["PqBundleKind.lastResort", "PqBundleKind.oneTimeBatch"];

pub fn variant_name(e: &CryptoEvent) -> &'static str {
    match e {
        CryptoEvent::MekRotated { .. } => "mekRotated",
        CryptoEvent::MekRequested { .. } => "mekRequested",
        CryptoEvent::MekTransferred { .. } => "mekTransferred",
        CryptoEvent::AdminKeypairGranted { .. } => "adminKeypairGranted",
        CryptoEvent::SlotKeypairGranted { .. } => "slotKeypairGranted",
        CryptoEvent::PqBundlePublished { .. } => "pqBundlePublished",
    }
}

fn kind_name(kind: &PqBundleKind) -> &'static str {
    match kind {
        PqBundleKind::LastResort => "PqBundleKind.lastResort",
        PqBundleKind::OneTimeBatch => "PqBundleKind.oneTimeBatch",
    }
}

pub fn nested_names(e: &CryptoEvent) -> Vec<&'static str> {
    match e {
        CryptoEvent::PqBundlePublished { kind, .. } => vec![kind_name(kind)],
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
            "pqBundlePublished.lastResort",
            CryptoEvent::PqBundlePublished {
                subkey: 5,
                kind: PqBundleKind::LastResort,
            },
        ),
        ev(
            "pqBundlePublished.oneTimeBatch",
            CryptoEvent::PqBundlePublished {
                subkey: 5,
                kind: PqBundleKind::OneTimeBatch,
            },
        ),
    ]
}
