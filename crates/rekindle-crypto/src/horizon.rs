//! `DELIVERY_HORIZON` (plan D24, ruling Q2b): how long a 1:1 message may
//! take to arrive. Every retention bound on the delivery path derives from
//! this one constant: the `unprocessed` age (`rekindle_db::repo::
//! unprocessed`), prekey grace (E2.4), the send log, inbox retention and
//! the outbox deadline (E2.5, D22).

use std::time::Duration;

/// 30 days: Signal's server message-queue retention (ruling Q2b,
/// 2026-10-19).
pub const DELIVERY_HORIZON: Duration = Duration::from_secs(30 * 24 * 60 * 60);
