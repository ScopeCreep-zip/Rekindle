//! D24's horizon inequalities (plan R3-X-1, ruling Q2b): every retention
//! bound on the 1:1 delivery path derives from the one
//! `rekindle_crypto::horizon::DELIVERY_HORIZON`, in this order:
//!
//! outbox deadline = HORIZON <= send log, prekey grace, inbox (chain)
//! retention, skipped-key age, each = HORIZON < unprocessed age = 1.5 x HORIZON.
//!
//! So a message the outbox still sends is still resendable from the send
//! log, still finds its prekey and its skipped key, and still sits on the
//! chain; and a row the receiver holds unprocessed outlives all of them.

use std::time::Duration;

use rekindle_crypto::horizon::DELIVERY_HORIZON;
use rekindle_crypto::signal::{dr::SKIPPED_KEY_MAX_AGE_MS, prekey_record::RETIRED_ARCHIVE_MS};
use rekindle_db::repo::unprocessed::UNPROCESSED_MAX_AGE;
use rekindle_db::repo::{chain_entries::CHAIN_RETENTION, send_log::SEND_LOG_MAX_AGE};
use rekindle_outbox::OUTBOX_DEADLINE;

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap()
}

#[test]
fn every_delivery_bound_derives_from_the_one_horizon() {
    let horizon = ms(DELIVERY_HORIZON);
    // Each derives from the constant, not a copy of its value.
    assert_eq!(ms(OUTBOX_DEADLINE), horizon);
    assert_eq!(ms(SEND_LOG_MAX_AGE), horizon);
    assert_eq!(ms(CHAIN_RETENTION), horizon);
    assert_eq!(u64::try_from(RETIRED_ARCHIVE_MS).unwrap(), horizon);
    assert_eq!(SKIPPED_KEY_MAX_AGE_MS, horizon);
    assert_eq!(ms(UNPROCESSED_MAX_AGE), horizon * 3 / 2);
}

/// The property: for every delay a message can take while the outbox still
/// sends it (0..=deadline, sampled across the whole range and at its edges),
/// each bound still holds it, and the receiver's unprocessed row outlives it.
#[test]
fn a_message_the_outbox_still_sends_is_held_everywhere_on_the_path() {
    let deadline = ms(OUTBOX_DEADLINE);
    let step = deadline / 997;
    let delays = (0..=997)
        .map(|i| i * step)
        .chain([1, deadline - 1, deadline]);
    for delay in delays {
        assert!(delay <= deadline);
        assert!(
            delay <= ms(SEND_LOG_MAX_AGE),
            "send log dropped it at {delay}"
        );
        assert!(
            delay <= u64::try_from(RETIRED_ARCHIVE_MS).unwrap(),
            "prekey gone at {delay}"
        );
        assert!(delay <= ms(CHAIN_RETENTION), "chain retired it at {delay}");
        assert!(
            delay <= SKIPPED_KEY_MAX_AGE_MS,
            "skipped key gone at {delay}"
        );
        assert!(
            delay < ms(UNPROCESSED_MAX_AGE),
            "unprocessed dropped it at {delay}"
        );
    }
    // Strict: the unprocessed age is the one bound past the horizon.
    assert!(ms(UNPROCESSED_MAX_AGE) > ms(DELIVERY_HORIZON));
}
