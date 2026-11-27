use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn preferences_round_trip_and_keep_the_rows_other_settings() {
    let mut conn = fixture::conn();
    assert_eq!(load(&conn, OWNER).unwrap(), Preferences::default());
    conn.execute(
        "INSERT INTO app_settings (owner_key, do_not_disturb) VALUES (?1, 1)",
        [OWNER],
    )
    .unwrap();
    let mut prefs = Preferences {
        input_device: Some("Mic".into()),
        auto_away_minutes: 0,
        ringtone_volume: 0.25,
        ..Preferences::default()
    };
    prefs.input_channels.insert("Mic".into(), vec![0, 2]);
    save(&mut conn, OWNER, &prefs).unwrap();
    assert_eq!(load(&conn, OWNER).unwrap(), prefs);
    let dnd: i64 = conn
        .query_row(
            "SELECT do_not_disturb FROM app_settings WHERE owner_key = ?1",
            [OWNER],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dnd, 1, "the row's other settings stay");
    prefs.input_channels.clear();
    save(&mut conn, OWNER, &prefs).unwrap();
    assert!(load(&conn, OWNER).unwrap().input_channels.is_empty());
}

#[test]
fn read_receipts_change_only_through_their_setter() {
    let mut conn = fixture::conn();
    assert!(!load(&conn, OWNER).unwrap().read_receipts_enabled);
    set_read_receipts_enabled(&conn, OWNER, true).unwrap();
    let mut prefs = load(&conn, OWNER).unwrap();
    assert!(prefs.read_receipts_enabled);
    prefs.read_receipts_enabled = false;
    prefs.auto_away_minutes = 5;
    save(&mut conn, OWNER, &prefs).unwrap();
    let stored = load(&conn, OWNER).unwrap();
    assert_eq!(stored.auto_away_minutes, 5);
    assert!(stored.read_receipts_enabled, "the bulk save leaves it");
    set_read_receipts_enabled(&conn, OWNER, false).unwrap();
    assert!(!load(&conn, OWNER).unwrap().read_receipts_enabled);
    // Sharing game totals: off by default, its own write only.
    assert!(!load(&conn, OWNER).unwrap().share_game_totals);
    set_share_game_totals(&conn, OWNER, true).unwrap();
    let mut prefs = load(&conn, OWNER).unwrap();
    assert!(prefs.share_game_totals);
    prefs.share_game_totals = false;
    save(&mut conn, OWNER, &prefs).unwrap();
    assert!(
        load(&conn, OWNER).unwrap().share_game_totals,
        "the bulk save leaves it"
    );
}

#[test]
fn the_skin_defaults_to_symbiosis_and_its_check_refuses_other_shapes() {
    let mut conn = fixture::conn();
    let defaults = SkinChoice {
        skin_id: "builtin:symbiosis".into(),
        sound_bank: "builtin:defaults".into(),
    };
    assert_eq!(skin_choice(&conn, OWNER).unwrap(), defaults);
    // A row made by another setting keeps the defaults.
    save(&mut conn, OWNER, &Preferences::default()).unwrap();
    assert_eq!(skin_choice(&conn, OWNER).unwrap(), defaults);
    set_skin_id(&conn, OWNER, "pkg:00").unwrap();
    assert_eq!(skin_choice(&conn, OWNER).unwrap().skin_id, "pkg:00");
    for bad in ["dev:00", "symbiosis", "", "Builtin:x"] {
        assert!(set_skin_id(&conn, OWNER, bad).is_err(), "{bad:?}");
    }
    for bad in ["dev:00", "defaults"] {
        let refused = conn.execute(
            "UPDATE app_settings SET sound_bank = ?2 WHERE owner_key = ?1",
            params![OWNER, bad],
        );
        assert!(refused.is_err(), "{bad:?}");
    }
    assert_eq!(skin_choice(&conn, OWNER).unwrap().skin_id, "pkg:00");
}

#[test]
fn an_out_of_range_value_is_refused() {
    let mut conn = fixture::conn();
    let prefs = Preferences {
        input_volume: 2.0,
        ..Preferences::default()
    };
    assert!(save(&mut conn, OWNER, &prefs).is_err());
}

/// Detection and showing the current game change only through their
/// own writes (plan E7.2): the desktop's bulk save cannot turn the
/// scanner on behind the Host, nor start sharing without its review.
#[test]
fn detection_and_current_game_sharing_change_only_through_their_setters() {
    let mut conn = fixture::conn();
    let prefs = load(&conn, OWNER).unwrap();
    assert!(prefs.game_detection_enabled);
    assert!(!prefs.share_current_game, "sharing is off by default");
    set_game_detection_enabled(&conn, OWNER, false).unwrap();
    set_share_current_game(&conn, OWNER, true).unwrap();
    let mut prefs = load(&conn, OWNER).unwrap();
    assert!(!prefs.game_detection_enabled);
    assert!(prefs.share_current_game);
    prefs.game_detection_enabled = true;
    prefs.share_current_game = false;
    save(&mut conn, OWNER, &prefs).unwrap();
    let stored = load(&conn, OWNER).unwrap();
    assert!(!stored.game_detection_enabled, "the bulk save leaves it");
    assert!(stored.share_current_game, "the bulk save leaves it");
}
