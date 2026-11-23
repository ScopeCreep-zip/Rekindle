//! The built-ins' stylesheet time, alone in its test binary as
//! `rekindle-governance-runtime/tests/walk_cpu_budget.rs` and
//! `rekindle-governance/tests/perf_targets.rs` are: cargo runs test
//! binaries one after another, so no sibling test loads the CPU while this
//! one measures it (beside the lib's other tests it failed at machine
//! loads of 30–60 and passed alone). The title's column work, the
//! regression this budget caught, is held by an operation count in the lib
//! (`regions/title_tests.rs`), which no load can move.

use std::time::{Duration, Instant};

use rekindle_skin::{builtin, serve, validate};

/// The time one window's stylesheet may take in a debug build: the Host
/// answers `SkinGet` and every `tokens.css` fetch with it, every window at
/// once on a skin change, inside the client's 5 s `DEFAULT_TIMEOUT`
/// (`rekindle-client` `client.rs`). Measured at e777t: at most about
/// 0.6 s; before, Xfire Classic took 2.8 s and xSLIMFIRE 3.6 s (the
/// title's per-column walk over every window width), so the windows kept
/// the previous skin and the skin list came back empty.
const SERVE_BUDGET: Duration = Duration::from_millis(1500);

#[test]
#[ignore = "wall-clock spec (1.5 s/pack, client 5 s timeout); cargo xtask perf"]
fn every_builtin_serves_its_stylesheet_in_time() {
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", pack.id));
        let address = c.address();
        let started = Instant::now();
        serve::stylesheet(&c.bytes, &address).expect("a stylesheet");
        let took = started.elapsed();
        assert!(took < SERVE_BUDGET, "{} served in {took:?}", pack.id);
    }
}
