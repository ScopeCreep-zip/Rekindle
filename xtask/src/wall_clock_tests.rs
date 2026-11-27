use super::*;

#[test]
fn flags_std_instants_through_imports_and_renames() {
    let src = "use std::time::{Duration, Instant};
                   use std::time::Instant as Wall;
                   use std::time;
                   fn a() { Instant::now(); Wall::now(); time::Instant::now(); std::time::Instant::now(); }";
    let hits = scan("crates/x/src/a.rs", src);
    assert_eq!(hits.len(), 4, "{hits:?}");
    assert!(hits[0].contains("a.rs:4"), "{hits:?}");
}

#[test]
fn passes_tokio_instants_and_flags_the_rest() {
    let src = "use tokio::time::Instant;
                   fn a() { Instant::now(); tokio::time::Instant::now().into_std(); }
                   fn b() { std::time::SystemTime::now(); chrono::Utc::now(); Local::now();
                            std::thread::sleep(d); tokio::time::sleep(d); }";
    let hits = scan("crates/x/src/a.rs", src);
    assert_eq!(hits.len(), 4, "{hits:?}");
    assert!(hits.iter().all(|h| !h.contains("  tokio")), "{hits:?}");
}

#[test]
fn flags_the_utils_wall_clock_helpers() {
    let src = "use rekindle_utils::timestamp_ms;
                   use rekindle_utils::time;
                   fn a() { timestamp_ms(); rekindle_utils::timestamp_secs(); time::timestamp_ms_i64();
                            clock.now_ms(); }";
    let hits = scan("crates/x/src/a.rs", src);
    assert_eq!(hits.len(), 3, "{hits:?}");
}

#[test]
fn flags_v7_uuids() {
    let src = "use uuid::Uuid;
                   fn a() { Uuid::now_v7(); uuid::Uuid::new_v7(ts); Uuid::new_v4(); }";
    let hits = scan("crates/x/src/a.rs", src);
    assert_eq!(hits.len(), 2, "{hits:?}");
}

#[test]
fn flags_re_exports_of_the_helpers_under_any_name() {
    let src = "pub use rekindle_utils::{timestamp_ms, timestamp_secs as now_secs};
                   pub(crate) use rekindle_utils::timestamp_ms_i64;
                   use rekindle_utils::Clock;
                   pub use rekindle_utils::Clock as C;";
    let hits = scan("crates/x/src/lib.rs", src);
    assert_eq!(hits.len(), 3, "{hits:?}");
}

#[test]
fn flags_reads_inside_macro_bodies() {
    let src = "use rekindle_utils::timestamp_ms;
                   use std::time::Instant;
                   fn a() {
                       tracing::info!(at = timestamp_ms(), \"x\");
                       let v = serde_json::json!({ \"t\": std::time::SystemTime::now() });
                       tokio::select! { _ = f() => { Instant::now(); } }
                       format!(\"{}\", tokio::time::Instant::now().elapsed().as_secs());
                   }";
    let hits = scan("crates/x/src/a.rs", src);
    assert_eq!(hits.len(), 3, "{hits:?}");
}

#[test]
fn ignores_test_code() {
    let src = "use std::time::Instant;
                   #[cfg(test)] mod tests { fn t() { std::time::Instant::now(); } }
                   #[test] fn t2() { Instant::now(); }";
    assert!(scan("crates/x/src/a.rs", src).is_empty());
}
