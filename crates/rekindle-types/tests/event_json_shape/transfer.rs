//! `TransferEvent` samples (plan E7.2 G12), covering every `TransferState`.

use rekindle_types::dto::transfer::TransferState;
use rekindle_types::subscription_events::{SubscriptionEvent, TransferEvent};

use crate::{id, Sample};

pub const NAMES: &[&str] = &["stateChanged", "progress", "downloadComplete", "removed"];

pub const NESTED: &[&str] = &[
    "TransferState.queued",
    "TransferState.active",
    "TransferState.paused",
    "TransferState.done",
    "TransferState.failed",
];

pub fn variant_name(e: &TransferEvent) -> &'static str {
    match e {
        TransferEvent::StateChanged { .. } => "stateChanged",
        TransferEvent::Progress { .. } => "progress",
        TransferEvent::DownloadComplete { .. } => "downloadComplete",
        TransferEvent::Removed { .. } => "removed",
    }
}

fn state_name(state: TransferState) -> &'static str {
    match state {
        TransferState::Queued => "TransferState.queued",
        TransferState::Active => "TransferState.active",
        TransferState::Paused => "TransferState.paused",
        TransferState::Done => "TransferState.done",
        TransferState::Failed => "TransferState.failed",
    }
}

pub fn nested_names(e: &TransferEvent) -> Vec<&'static str> {
    match e {
        TransferEvent::StateChanged { state, .. } => vec![state_name(*state)],
        TransferEvent::Progress { .. }
        | TransferEvent::DownloadComplete { .. }
        | TransferEvent::Removed { .. } => vec![],
    }
}

fn attachment() -> String {
    id("att_", 'a')
}

fn ev(name: &str, e: TransferEvent) -> Sample {
    (format!("transfer.{name}"), SubscriptionEvent::Transfer(e))
}

fn state(suffix: &str, state: TransferState) -> Sample {
    ev(
        &format!("stateChanged.{suffix}"),
        TransferEvent::StateChanged {
            attachment_id: attachment(),
            state,
        },
    )
}

pub fn samples() -> Vec<Sample> {
    vec![
        state("queued", TransferState::Queued),
        state("active", TransferState::Active),
        state("paused", TransferState::Paused),
        state("done", TransferState::Done),
        state("failed", TransferState::Failed),
        ev(
            "progress",
            TransferEvent::Progress {
                attachment_id: attachment(),
                chunks_done: 3,
                chunk_count: 8,
                bytes_done: 3 * 262_144,
                total: 2_000_000,
            },
        ),
        ev(
            "downloadComplete",
            TransferEvent::DownloadComplete {
                attachment_id: attachment(),
            },
        ),
        ev(
            "removed",
            TransferEvent::Removed {
                attachment_id: attachment(),
            },
        ),
    ]
}
