//! The identity's preferences (plan D1): the `app_settings` preference
//! columns and `app_input_channels`. A missing row is the defaults.

use std::collections::BTreeMap;

use rekindle_types::preferences::{NotificationContent, Preferences};
use rusqlite::{params, Connection, OptionalExtension as _};

/// The identity's preferences; the defaults when none were saved.
///
/// # Errors
/// The query failed.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<Preferences> {
    let stored = conn
        .query_row(
            "SELECT notifications_enabled, notification_sound, start_minimized, \
             game_detection_enabled, game_scan_interval_secs, input_device, output_device, \
             video_device_id, video_device_label, input_volume, output_volume, \
             noise_suppression, echo_cancellation, auto_away_minutes, \
             auto_volunteer_relay_for_new_friends, ringtone_enabled, ringtone_volume, \
             in_call_dnd_auto_enable, read_receipts_enabled, notification_content, \
             share_game_totals, share_current_game, keep_activity_log \
             FROM app_settings WHERE owner_key = ?1",
            params![owner_key],
            |row| {
                Ok(Preferences {
                    notifications_enabled: row.get(0)?,
                    notification_sound: row.get(1)?,
                    start_minimized: row.get(2)?,
                    game_detection_enabled: row.get(3)?,
                    game_scan_interval_secs: row.get(4)?,
                    input_device: row.get(5)?,
                    output_device: row.get(6)?,
                    video_device_id: row.get(7)?,
                    video_device_label: row.get(8)?,
                    input_volume: row.get(9)?,
                    output_volume: row.get(10)?,
                    noise_suppression: row.get(11)?,
                    echo_cancellation: row.get(12)?,
                    auto_away_minutes: row.get(13)?,
                    auto_volunteer_relay_for_new_friends: row.get(14)?,
                    ringtone_enabled: row.get(15)?,
                    ringtone_volume: row.get(16)?,
                    in_call_dnd_auto_enable: row.get(17)?,
                    read_receipts_enabled: row.get(18)?,
                    notification_content: {
                        let text: String = row.get(19)?;
                        NotificationContent::from_stored(&text).ok_or_else(|| {
                            rusqlite::Error::FromSqlConversionFailure(
                                19,
                                rusqlite::types::Type::Text,
                                format!("notification_content {text:?}").into(),
                            )
                        })?
                    },
                    share_game_totals: row.get(20)?,
                    share_current_game: row.get(21)?,
                    keep_activity_log: row.get(22)?,
                    input_channels: BTreeMap::new(),
                })
            },
        )
        .optional()?;
    let mut prefs = stored.unwrap_or_default();
    let mut stmt = conn.prepare(
        "SELECT device_name, channel FROM app_input_channels WHERE owner_key = ?1 \
         ORDER BY device_name, channel",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u16>(1)?))
    })?;
    for row in rows {
        let (device, channel) = row?;
        prefs
            .input_channels
            .entry(device)
            .or_default()
            .push(channel);
    }
    Ok(prefs)
}

/// Save the identity's preferences: the preference columns (the row's
/// other settings stay) and the input channels, in one transaction.
/// `read_receipts_enabled`, `share_game_totals`, `share_current_game` and
/// `game_detection_enabled` are not saved here: each changes only through
/// its own request's write ([`set_read_receipts_enabled`],
/// [`set_share_game_totals`], [`set_share_current_game`],
/// [`set_game_detection_enabled`]), which the Host reviews and acts on.
///
/// # Errors
/// A write failed, or a value is out of its column's range.
pub fn save(conn: &mut Connection, owner_key: &str, prefs: &Preferences) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO app_settings (owner_key, notifications_enabled, notification_sound, \
         start_minimized, game_scan_interval_secs, input_device, \
         output_device, video_device_id, video_device_label, input_volume, output_volume, \
         noise_suppression, echo_cancellation, auto_away_minutes, \
         auto_volunteer_relay_for_new_friends, ringtone_enabled, ringtone_volume, \
         in_call_dnd_auto_enable, notification_content) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
         ON CONFLICT(owner_key) DO UPDATE SET \
         notifications_enabled = excluded.notifications_enabled, \
         notification_sound = excluded.notification_sound, \
         start_minimized = excluded.start_minimized, \
         game_scan_interval_secs = excluded.game_scan_interval_secs, \
         input_device = excluded.input_device, \
         output_device = excluded.output_device, \
         video_device_id = excluded.video_device_id, \
         video_device_label = excluded.video_device_label, \
         input_volume = excluded.input_volume, \
         output_volume = excluded.output_volume, \
         noise_suppression = excluded.noise_suppression, \
         echo_cancellation = excluded.echo_cancellation, \
         auto_away_minutes = excluded.auto_away_minutes, \
         auto_volunteer_relay_for_new_friends = excluded.auto_volunteer_relay_for_new_friends, \
         ringtone_enabled = excluded.ringtone_enabled, \
         ringtone_volume = excluded.ringtone_volume, \
         in_call_dnd_auto_enable = excluded.in_call_dnd_auto_enable, \
         notification_content = excluded.notification_content",
        params![
            owner_key,
            prefs.notifications_enabled,
            prefs.notification_sound,
            prefs.start_minimized,
            prefs.game_scan_interval_secs,
            prefs.input_device,
            prefs.output_device,
            prefs.video_device_id,
            prefs.video_device_label,
            prefs.input_volume,
            prefs.output_volume,
            prefs.noise_suppression,
            prefs.echo_cancellation,
            prefs.auto_away_minutes,
            prefs.auto_volunteer_relay_for_new_friends,
            prefs.ringtone_enabled,
            prefs.ringtone_volume,
            prefs.in_call_dnd_auto_enable,
            prefs.notification_content.as_str(),
        ],
    )?;
    tx.execute(
        "DELETE FROM app_input_channels WHERE owner_key = ?1",
        params![owner_key],
    )?;
    for (device, channels) in &prefs.input_channels {
        for channel in channels {
            tx.execute(
                "INSERT OR IGNORE INTO app_input_channels (owner_key, device_name, channel) \
                 VALUES (?1, ?2, ?3)",
                params![owner_key, device, channel],
            )?;
        }
    }
    tx.commit()
}

/// Turn DM read receipts on or off (`SetReadReceiptsEnabled`, plan E2.5);
/// the row's other settings stay.
///
/// # Errors
/// The write failed.
pub fn set_read_receipts_enabled(
    conn: &Connection,
    owner_key: &str,
    enabled: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, read_receipts_enabled) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET read_receipts_enabled = excluded.read_receipts_enabled",
        params![owner_key, enabled],
    )?;
    Ok(())
}

/// Turn sharing game totals with friends on or off (`SetShareGameTotals`,
/// plan E7.2); the row's other settings stay.
///
/// # Errors
/// The write failed.
pub fn set_share_game_totals(
    conn: &Connection,
    owner_key: &str,
    enabled: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, share_game_totals) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET share_game_totals = excluded.share_game_totals",
        params![owner_key, enabled],
    )?;
    Ok(())
}

/// Turn game detection on or off (`SetGameDetection`, plan E7.2); the
/// row's other settings stay.
///
/// # Errors
/// The write failed.
pub fn set_game_detection_enabled(
    conn: &Connection,
    owner_key: &str,
    enabled: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, game_detection_enabled) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET \
         game_detection_enabled = excluded.game_detection_enabled",
        params![owner_key, enabled],
    )?;
    Ok(())
}

/// Turn showing our current game to friends on or off
/// (`SetShareCurrentGame`, plan E7.2); the row's other settings stay.
///
/// # Errors
/// The write failed.
pub fn set_share_current_game(
    conn: &Connection,
    owner_key: &str,
    enabled: bool,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, share_current_game) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET share_current_game = excluded.share_current_game",
        params![owner_key, enabled],
    )?;
    Ok(())
}

/// Keep the activity feed or stop (`SetKeepActivityLog`, plan E7.15); the
/// row's other settings stay. Off clears `activity_log` in the same
/// transaction, so no entry outlives the choice.
///
/// # Errors
/// A write failed; nothing is kept.
pub fn set_keep_activity_log(
    conn: &mut Connection,
    owner_key: &str,
    enabled: bool,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO app_settings (owner_key, keep_activity_log) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET keep_activity_log = excluded.keep_activity_log",
        params![owner_key, enabled],
    )?;
    if !enabled {
        crate::repo::activity::clear(&tx, owner_key)?;
    }
    tx.commit()
}

/// The identity's skin and sound bank (plan E7.7.2), each a
/// `builtin:<id>` or `pkg:<hash>` id. Device-bound: never synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinChoice {
    pub skin_id: String,
    pub sound_bank: String,
}

/// The identity's skin and sound bank; the column defaults when no row
/// was saved.
///
/// # Errors
/// The query failed.
pub fn skin_choice(conn: &Connection, owner_key: &str) -> rusqlite::Result<SkinChoice> {
    let stored = conn
        .query_row(
            "SELECT skin_id, sound_bank FROM app_settings WHERE owner_key = ?1",
            params![owner_key],
            |row| {
                Ok(SkinChoice {
                    skin_id: row.get(0)?,
                    sound_bank: row.get(1)?,
                })
            },
        )
        .optional()?;
    match stored {
        Some(choice) => Ok(choice),
        None => conn.query_row(
            "SELECT \
             (SELECT dflt_value FROM pragma_table_info('app_settings') WHERE name = 'skin_id'), \
             (SELECT dflt_value FROM pragma_table_info('app_settings') WHERE name = 'sound_bank')",
            [],
            |row| {
                let unquote = |v: String| v.trim_matches('\'').to_owned();
                Ok(SkinChoice {
                    skin_id: unquote(row.get(0)?),
                    sound_bank: unquote(row.get(1)?),
                })
            },
        ),
    }
}

/// Set the identity's skin; the row's other settings stay. The caller has
/// checked that `skin_id` names a built-in or a `skin_packages` row; the
/// column's CHECK refuses any other shape.
///
/// # Errors
/// The write failed or the CHECK refused the id.
pub fn set_skin_id(conn: &Connection, owner_key: &str, skin_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, skin_id) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET skin_id = excluded.skin_id",
        params![owner_key, skin_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
}

/// The quick wipe's panic shortcut (plan E7.1), `None` while the shortcut
/// is unarmed.
///
/// # Errors
/// The read failed.
pub fn panic_shortcut(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT panic_shortcut FROM app_settings WHERE owner_key = ?1",
            params![owner_key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// Set or clear the panic shortcut (`ArmQuickWipe` / `DisarmQuickWipe`,
/// plan E7.1); the row's other settings stay.
///
/// # Errors
/// The write failed (the CHECK refuses a malformed accelerator).
pub fn set_panic_shortcut(
    conn: &Connection,
    owner_key: &str,
    shortcut: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_settings (owner_key, panic_shortcut) VALUES (?1, ?2) \
         ON CONFLICT(owner_key) DO UPDATE SET panic_shortcut = excluded.panic_shortcut",
        params![owner_key, shortcut],
    )?;
    Ok(())
}

#[cfg(test)]
mod panic_shortcut_tests {
    use super::{panic_shortcut, set_panic_shortcut};
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn the_panic_shortcut_round_trips_and_refuses_a_malformed_accelerator() {
        let conn = fixture::conn();
        assert_eq!(panic_shortcut(&conn, OWNER).unwrap(), None);
        set_panic_shortcut(&conn, OWNER, Some("CommandOrControl+Shift+F12")).unwrap();
        assert_eq!(
            panic_shortcut(&conn, OWNER).unwrap().as_deref(),
            Some("CommandOrControl+Shift+F12")
        );
        for bad in ["F12", "Ctrl+", "Ctrl+Shift+F12; DROP", "+X"] {
            assert!(
                set_panic_shortcut(&conn, OWNER, Some(bad)).is_err(),
                "{bad}"
            );
        }
        set_panic_shortcut(&conn, OWNER, None).unwrap();
        assert_eq!(panic_shortcut(&conn, OWNER).unwrap(), None);
    }
}
