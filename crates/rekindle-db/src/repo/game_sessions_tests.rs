use super::*;
use crate::repo::fixture::{conn, OWNER};

#[test]
fn totals_sum_each_game_and_order_by_last_played() {
    let conn = conn();
    record(&conn, OWNER, 4181, 0, 60_000).unwrap();
    record(&conn, OWNER, 4102, 70_000, 100_000).unwrap();
    record(&conn, OWNER, 4181, 200_000, 290_500).unwrap();
    assert_eq!(
        totals(&conn, OWNER).unwrap(),
        vec![
            GameTotal {
                game_id: 4181,
                total_secs: 150,
                last_played: 290_500
            },
            GameTotal {
                game_id: 4102,
                total_secs: 30,
                last_played: 100_000
            },
        ]
    );
    assert!(totals(&conn, "someone-else").unwrap().is_empty());
}

#[test]
fn a_session_ending_before_it_starts_or_with_no_game_is_refused() {
    let conn = conn();
    assert!(record(&conn, OWNER, 4181, 10, 9).is_err());
    assert!(record(&conn, OWNER, 0, 0, 1).is_err());
    assert!(record(&conn, "no-such-identity", 4181, 0, 1).is_err());
}
