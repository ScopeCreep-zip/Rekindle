//! Event-driven route lifecycle policy.
//!
//! Routes are NOT rotated on a timer: veilid-core manages route health
//! itself (its private_route_management task tests routes and reports
//! death via `RouteChange`), and a fixed rotation interval is also a
//! deterministic-timing fingerprint. A route lives until Veilid names
//! it dead or attachment is lost; death triggers an immediate reallocation
//! by the route's owner (`rekindle_protocol::own_routes`, single-flight per
//! class, so a `RouteChange` burst cannot stack heals). These constants
//! define that policy identically for every process (Tauri host,
//! daemon/CLI transport) — backend-owns-policy.

use std::time::Duration;

/// Cadence of the attached-but-routeless watchdog — the only timer in
/// the route lifecycle. It backstops missed heals (allocation failures,
/// startup races, reattach), never rotates a live route.
pub const ROUTE_WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);

/// Backstop TTL for cached PEER route blobs. Peers no longer rotate on
/// a timer; their blobs stay valid until dead-remote-route events or
/// send failures invalidate them. This guards against silently-dead
/// entries only.
pub const PEER_ROUTE_CACHE_MAX_AGE: Duration = Duration::from_secs(900);
