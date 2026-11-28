use super::*;
use crate::repo::fixture::{self, OWNER};

/// A bucket saved after a spend reads back unchanged, and before the
/// first unlock there is none.
#[test]
fn a_saved_bucket_reads_back_unchanged() {
    let conn = fixture::conn();
    assert_eq!(load(&conn, OWNER, BudgetKind::Routes).unwrap(), None);
    let row = (11, 1_763_850_000_000, 1_763_500_000_000, 13);
    save(&conn, OWNER, BudgetKind::Routes, row).unwrap();
    assert_eq!(load(&conn, OWNER, BudgetKind::Routes).unwrap(), Some(row));
}

/// The code slot's roll budget round-trips in its own row: a spent one
/// reads back spent (a restart never refills it), and neither bucket's
/// save touches the other's.
#[test]
fn a_spent_code_roll_bucket_reads_back_apart_from_the_routes() {
    let conn = fixture::conn();
    let routes = (24, 1_763_850_000_000, 1_763_500_000_000, 0);
    save(&conn, OWNER, BudgetKind::Routes, routes).unwrap();
    assert_eq!(load(&conn, OWNER, BudgetKind::CodeRolls).unwrap(), None);
    let spent = (0, 1_763_860_000_000, 1_763_500_000_000, 24);
    save(&conn, OWNER, BudgetKind::CodeRolls, spent).unwrap();
    assert_eq!(
        load(&conn, OWNER, BudgetKind::CodeRolls).unwrap(),
        Some(spent)
    );
    assert_eq!(
        load(&conn, OWNER, BudgetKind::Routes).unwrap(),
        Some(routes)
    );
}

/// A stamp SQLite can't hold as `i64` fails the write instead of
/// wrapping into a negative (and later unreadable) value.
#[test]
fn a_stamp_past_i64_fails_the_write() {
    let conn = fixture::conn();
    assert!(save(&conn, OWNER, BudgetKind::Routes, (1, u64::MAX, 0, 0)).is_err());
    assert_eq!(load(&conn, OWNER, BudgetKind::Routes).unwrap(), None);
}
