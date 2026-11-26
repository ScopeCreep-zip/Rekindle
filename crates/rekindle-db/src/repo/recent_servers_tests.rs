use super::*;
use crate::repo::fixture::{self, OWNER};

fn visit(game_id: u32, address: &str, last_seen: i64) -> RecentServer {
    RecentServer {
        game_id,
        address: address.to_owned(),
        last_seen,
    }
}

#[test]
fn a_revisit_moves_the_row_and_the_oldest_past_the_cap_goes() {
    let conn = fixture::conn();
    let cap = usize::try_from(MAX_RECENT).unwrap();
    for i in 0..i64::from(MAX_RECENT) {
        touch(
            &conn,
            OWNER,
            &visit(10, &format!("1.2.3.4:{}", 1000 + i), i),
        )
        .unwrap();
    }
    touch(&conn, OWNER, &visit(10, "1.2.3.4:1000", 500)).unwrap();
    touch(&conn, OWNER, &visit(11, "5.6.7.8:1", 600)).unwrap();
    let rows = list(&conn, OWNER).unwrap();
    assert_eq!(rows.len(), cap);
    assert_eq!(rows[0], visit(11, "5.6.7.8:1", 600));
    assert_eq!(rows[1], visit(10, "1.2.3.4:1000", 500));
    assert!(
        !rows.iter().any(|r| r.address == "1.2.3.4:1001"),
        "the oldest stays"
    );
    remove(&conn, OWNER, 11, "5.6.7.8:1").unwrap();
    assert_eq!(list(&conn, OWNER).unwrap().len(), cap - 1);
    clear(&conn, OWNER).unwrap();
    assert!(list(&conn, OWNER).unwrap().is_empty());
}
