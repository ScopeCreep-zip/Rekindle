use super::*;
use crate::repo::fixture::{self, OWNER};

fn fav(game_id: u32, address: &str, label: &str, added_at: i64) -> FavouriteServer {
    FavouriteServer {
        game_id,
        address: address.into(),
        label: label.into(),
        added_at,
    }
}

/// Plan E7.2: adding again relabels in place, removal is idempotent,
/// and the table refuses game 0 and a label over 64 characters.
#[test]
fn add_relabels_remove_is_idempotent_and_checks_hold() {
    let conn = fixture::conn();
    add(&conn, OWNER, &fav(4181, "203.0.113.7:27015", "LAN", 10)).unwrap();
    add(&conn, OWNER, &fav(12, "example.net:25565", "", 20)).unwrap();
    add(&conn, OWNER, &fav(4181, "203.0.113.7:27015", "Scrims", 30)).unwrap();
    assert_eq!(
        list(&conn, OWNER).unwrap(),
        vec![
            fav(4181, "203.0.113.7:27015", "Scrims", 10),
            fav(12, "example.net:25565", "", 20),
        ]
    );
    remove(&conn, OWNER, 4181, "203.0.113.7:27015").unwrap();
    remove(&conn, OWNER, 4181, "203.0.113.7:27015").unwrap();
    assert_eq!(list(&conn, OWNER).unwrap().len(), 1);
    assert!(
        add(&conn, OWNER, &fav(0, "a.b:1", "", 1)).is_err(),
        "game 0"
    );
    assert!(
        add(&conn, OWNER, &fav(7, "a.b:1", &"x".repeat(65), 1)).is_err(),
        "label over 64"
    );
}
