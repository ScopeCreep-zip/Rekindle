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

/// Why a route class is down with no new route allocated (plan RV1, threat
/// model L2): fail-closed in both cases, never a silent redraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutePauseCause {
    /// The identity's new-route budget is spent: "inbound route paused
    /// after repeated failures: possible attack".
    Exhausted,
    /// The identity's stored budget could not be read: no new route is paid
    /// for until it can be (never a full budget in its place).
    Unreadable,
}

/// One own route class's outage (plan RV1g): when the class went down, in
/// epoch milliseconds, and the network's latest reason. A window shows
/// "Unreachable — retrying since <clock time>".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnRouteOutage {
    pub since_ms: u64,
    pub reason: String,
}

/// Which own route classes are down now (plan RV1g), as the network
/// status carries it, so a window opened mid-outage shows it; later
/// changes arrive as [`NetworkEvent::OwnRouteDown`] / `OwnRouteUp`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnRouteDownSet {
    pub general: Option<OwnRouteOutage>,
    pub media: Option<OwnRouteOutage>,
}

impl OwnRouteDownSet {
    /// The class's outage, if it is down.
    #[must_use]
    pub fn get(&self, class: OwnRouteClass) -> Option<&OwnRouteOutage> {
        match class {
            OwnRouteClass::General => self.general.as_ref(),
            OwnRouteClass::Media => self.media.as_ref(),
        }
    }

    /// The class's entry, to set or clear.
    pub fn entry(&mut self, class: OwnRouteClass) -> &mut Option<OwnRouteOutage> {
        match class {
            OwnRouteClass::General => &mut self.general,
            OwnRouteClass::Media => &mut self.media,
        }
    }
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
    /// A route class is down with no new route allocated (plan RV1, threat
    /// model L2): for `Exhausted` a window shows "inbound route paused after
    /// repeated failures: possible attack"; for `Unreadable` that the stored
    /// budget could not be read. Never a silent redraw. A pause always has
    /// its cause; [`NetworkEvent::RouteResumed {}`] ends it.
    /// Triggered by: the Host following `RoutePort::own_paused`.
    RoutePaused { cause: RoutePauseCause },
    /// The pause ended: a refill paid for a new route (plan RV1).
    /// Triggered by: the Host following `RoutePort::own_paused`.
    RouteResumed {},
    /// Own-route deaths of `class` within `window_secs` rose above the
    /// baseline (`baseline`): a warning only, with no automatic reaction
    /// (plan RV1). `young` of them died younger than the young-route age
    /// (`rekindle_route::budget::YOUNG_ROUTE_AGE`: Veilid's own judgement
    /// of a route that never passed a test). Triggered by: the Host's
    /// per-class death counts.
    RouteDeathsAboveBaseline {
        class: OwnRouteClass,
        deaths: u32,
        young: u32,
        window_secs: u64,
        baseline: u32,
    },
    /// One of our route classes went down and its replacement keeps
    /// failing (plan RV1g): sent once on the down transition, never per
    /// attempt. `since_ms` is when the class went down (epoch ms).
    /// Triggered by: the Host following `RoutePort::own_down`.
    OwnRouteDown {
        class: OwnRouteClass,
        since_ms: u64,
        reason: String,
    },
    /// The class is up again (plan RV1g): sent once, after an
    /// `OwnRouteDown`. Triggered by: the Host following `RoutePort::own_down`.
    OwnRouteUp { class: OwnRouteClass },
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
