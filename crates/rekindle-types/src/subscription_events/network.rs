//! Network and infrastructure events — attachment, routes, DHT watches.

use serde::{Deserialize, Serialize};

/// What one of our route classes is doing, for a status indicator (plan
/// C7.9c). Mirrors the route owner's state without its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteAvailability {
    /// Not wanted yet (no node, or before the first want).
    Idle,
    /// An allocation is in flight.
    Allocating,
    /// A route is live and published.
    Available,
    /// Veilid said try again; a retry is scheduled.
    Unavailable,
    /// Allocation failed for good; the watchdog's backstop want retries.
    Failed,
}

/// One of our own route classes, as a route event names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OwnRouteClass {
    /// Messages, calls and the published profile/mailbox route.
    General,
    /// Realtime media.
    Media,
}

/// Where the unlocked identity's own-records stage is (plan E2.5 FC3):
/// whether our profile and mailbox are published yet. Inviting, adding or
/// accepting a friend and a first message name them, so each waits for
/// `Held`; a window shows the wait while it lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OwnRecordsPhase {
    /// Still publishing (or not started since the unlock).
    Pending,
    /// Published: the operations that name them go ahead.
    Held,
    /// The stage ended without them; the event's `error` says why.
    Failed,
}

/// Network infrastructure events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NetworkEvent {
    /// Network attachment state changed (attached, detached, degraded).
    /// Triggered by: `VeilidUpdate::Attachment` via dispatch loop.
    AttachmentChanged {
        /// Raw Veilid `AttachmentState` string, e.g. `"detached"`,
        /// `"attaching"`, `"attached_good"`. The booleans below collapse
        /// this to two bits; the desktop's network indicator needs the
        /// distinction between "attaching" and "attached_weak", which
        /// neither bit can express.
        attachment_state: String,
        is_attached: bool,
        public_internet_ready: bool,
        /// Whether we hold an allocated private route for receiving.
        /// Attached with no route means nobody can reach us, which is
        /// not visible from the other three fields.
        has_route: bool,
        /// What our media-class route is doing. Voice and video need it,
        /// and the general route is never substituted (plan C7.9c), so a
        /// window must be able to say voice is unavailable.
        media_route: RouteAvailability,
        /// Whether the Host accepts writes now (its lifecycle's
        /// `can_write`: unlocked and not detached). A community create
        /// waits for it and `public_internet_ready` (plan E7.13, audit
        /// 2026-11-18): the anchor's owner key is destroyed after the
        /// GenesisCert (ADR 0011), so a create cannot be held for later.
        can_write: bool,
    },
    /// The network indicator's bars, 0 to 5 (plan E7.2, live finding L8):
    /// Veilid's attachment strength (`AttachmentState::bars`) smoothed by
    /// the Host, so a short dip does not flicker the indicator and every
    /// frontend shows the same bars. Sent when the shown value changes.
    Strength { bars: u8 },
    /// The own-records stage moved (plan E2.5 FC3): published at the
    /// unlock (`Pending`) and when it ends (`Held` or `Failed` with the
    /// reason). A window opened later reads it from the network status.
    /// Triggered by: the Host's own-records stage.
    OwnRecordsChanged {
        phase: OwnRecordsPhase,
        error: Option<String>,
    },
    /// Our own allocated private routes died and need reallocation.
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_routes).
    LocalRoutesDied { count: usize },
    /// The identity's new-route budget is spent, so a route class stays
    /// down (`paused: true`), or a refill brought it back (plan RV1, threat
    /// model L2): a window shows "inbound route paused after repeated
    /// failures: possible attack". Never a silent redraw.
    /// Triggered by: the Host following `RoutePort::own_paused`.
    RoutePaused { paused: bool },
    /// Own-route deaths of `class` within `window_secs` rose above the
    /// baseline (`baseline`): a warning only, with no automatic reaction
    /// (plan RV1). Triggered by: the Host's per-class death counts.
    RouteDeathsAboveBaseline {
        class: OwnRouteClass,
        deaths: u32,
        window_secs: u64,
        baseline: u32,
    },
    /// Remote peer routes died (imported routes expired).
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_remote_routes).
    RemoteRoutesDied { peer_keys: Vec<String> },
    /// A DHT record's value changed (generic, for records without specific handlers).
    /// Triggered by: `VeilidUpdate::ValueChange` for unregistered record keys.
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },
}
