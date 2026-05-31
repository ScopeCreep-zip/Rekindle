//! Phase 23.D.3 — community notifications subsystem decomposed into
//! focused submodules per Invariant 1 (≤500 LoC/file):
//!
//! * `types`       — `NotificationThrottle` + `NotificationDecision` +
//!   `NotificationLevel` + `QuietHoursSettings` + `CleartextMentions`
//!   + throttle window math + tests.
//! * `level`       — per-channel + community-default + tier resolver
//!   for `NotificationLevel`.
//! * `sound`       — per-channel + community-default sound resolver +
//!   setter (BLAKE3 ref of a soundboard expression).
//! * `quiet_hours` — global DND + quiet-hours window with IANA tz.
//! * `emit`        — final emit pipeline: should-emit gate (DND +
//!   quiet hours + mention rules) and `emit_message_notification`
//!   throttle/burst-summary fan-out.

mod emit;
mod level;
mod quiet_hours;
mod sound;
mod types;

pub use emit::{emit_message_notification, should_emit_message_notification};
pub use level::{
    get_community_default_notification_level, parse_notification_level,
    set_channel_notification_level, set_community_default_notification_level,
};
pub use quiet_hours::{get_quiet_hours, is_do_not_disturb_active, set_do_not_disturb, set_quiet_hours};
pub use sound::{resolve_notification_sound, set_notification_sound};
pub use types::{
    CleartextMentions, NotificationDecision, NotificationLevel, NotificationThrottle,
    QuietHoursSettings,
};
