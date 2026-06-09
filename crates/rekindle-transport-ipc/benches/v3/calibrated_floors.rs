//! Calibrated irreducible floor benchmarks.
//!
//! Every measurement is identified by an `osc:` identifier, dependency-gated
//! by the calibrate module's accumulative order, and emitted as a structured
//! JSONL record to `target/calibration.jsonl`.
//!
//! Uses `fn main()` instead of `criterion_main!` because `CalibratedSession`
//! must persist across all benchmark groups. `criterion_main!` generates its
//! own `main` which doesn't allow session injection.
//!
//! Run: `cargo bench -p rekindle-transport-ipc --bench v3_calibrated_floors`
//! Filter: `cargo bench ... -- "dram_bandwidth"` (criterion pattern match)
//! Tracing: `RUST_LOG=info cargo bench ...` (shows dep checks, skips, records)

#![allow(unsafe_code)]

mod floors;

use rekindle_transport_ipc::calibrate::{
    CalibratedSession, Profile,
    init_tracing, calibrated_criterion, finalize,
};

fn main() {
    init_tracing();
    tracing::info!("calibrated floors: starting");

    let mut criterion = calibrated_criterion();

    let mut session = CalibratedSession::new(Profile::L4Validation);
    tracing::info!(
        profile = ?session.profile(),
        substrate = %session.substrate().canonical(),
        "calibrated floors: session created"
    );

    // Tier 0 — pure memory, no kernel, no threads
    tracing::info!("calibrated floors: tier 0 — memory primitives");
    floors::mem::register(&mut criterion, &mut session);

    // Tier 1 — kernel primitives (io_uring NOP, mmap)
    tracing::info!("calibrated floors: tier 1 — kernel primitives");
    floors::io::register(&mut criterion, &mut session);

    // Tier 2 — cross-thread synchronization
    tracing::info!("calibrated floors: tier 2 — synchronization");
    floors::sync::register(&mut criterion, &mut session);

    // Tier 3 — crypto primitives
    tracing::info!("calibrated floors: tier 3 — crypto");
    floors::crypto::register(&mut criterion, &mut session);

    // Tier 4 — scheduler primitives
    tracing::info!("calibrated floors: tier 4 — scheduler");
    floors::sched::register(&mut criterion, &mut session);

    // Tier 5 — pool primitives
    tracing::info!("calibrated floors: tier 5 — pool");
    floors::pool::register(&mut criterion, &mut session);

    // Backfill, baseline, JSONL, final_summary — all in one call.
    finalize(&mut session, &mut criterion, "floors");
}
