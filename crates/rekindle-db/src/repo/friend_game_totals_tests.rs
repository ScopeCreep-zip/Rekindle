use super::*;
use crate::repo::fixture::{conn, OWNER};

fn total(game_id: u32, total_secs: u64, last_played: i64) -> GameTotal {
    GameTotal {
        game_id,
        total_secs,
        last_played,
    }
}

#[test]
fn a_frame_replaces_the_totals_and_unfriending_removes_them() {
    let mut conn = conn();
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
    replace(
        &mut conn,
        OWNER,
        "bob",
        &[total(4181, 60, 10), total(4102, 5, 20)],
    )
    .unwrap();
    assert_eq!(
        of(&conn, OWNER, "bob").unwrap(),
        [total(4102, 5, 20), total(4181, 60, 10)]
    );
    replace(&mut conn, OWNER, "bob", &[total(4181, 90, 30)]).unwrap();
    assert_eq!(of(&conn, OWNER, "bob").unwrap(), [total(4181, 90, 30)]);
    assert!(
        replace(&mut conn, OWNER, "carol", &[total(1, 1, 1)]).is_err(),
        "only a friend's totals are kept"
    );
    conn.execute("DELETE FROM friends WHERE public_key = 'bob'", [])
        .unwrap();
    assert!(of(&conn, OWNER, "bob").unwrap().is_empty());
}
