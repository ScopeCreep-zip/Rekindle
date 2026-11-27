use super::{key_tables, scan};

fn tables() -> Vec<String> {
    key_tables("pub const KEY_MATERIAL_TABLES: [&str; 2] = [\n \"ratchet_sessions\",\n \"signal_prekeys\",\n];")
}

#[test]
fn a_key_delete_outside_the_helper_is_refused() {
    let raw = "fn f(c: &C) { c.execute(\"DELETE FROM ratchet_sessions WHERE x = 1\", []); }";
    assert_eq!(scan("crates/x/src/lib.rs", raw, &tables()).len(), 1);
    let split = "fn f(c: &C) { c.execute(\"DELETE FROM signal_prekeys \\\n WHERE x\", []); }";
    assert_eq!(scan("crates/x/src/lib.rs", split, &tables()).len(), 1);
}

#[test]
fn the_helper_and_other_tables_pass() {
    let helper = "fn f(c: &C) { crate::erase::delete(c, \"DELETE FROM ratchet_sessions\", []); }";
    assert!(scan("crates/x/src/lib.rs", helper, &tables()).is_empty());
    let other = "fn f(c: &C) { c.execute(\"DELETE FROM ratchet_sessions_log\", []); }";
    assert!(scan("crates/x/src/lib.rs", other, &tables()).is_empty());
    let test = "#[cfg(test)] mod t { fn f() { x(\"DELETE FROM ratchet_sessions\"); } }";
    assert!(scan("crates/x/src/lib.rs", test, &tables()).is_empty());
}
