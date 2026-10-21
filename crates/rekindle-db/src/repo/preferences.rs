//! The identity's preferences (plan D1): the `app_settings` preference
//! columns and `app_input_channels`. A missing row is the defaults.

use std::collections::BTreeMap;

use rekindle_types::preferences::Preferences;
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
             in_call_dnd_auto_enable, read_receipts_enabled \
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
/// `read_receipts_enabled` is not saved here: it changes only through
/// [`set_read_receipts_enabled`], the trust-changing intent's write.
///
/// # Errors
/// A write failed, or a value is out of its column's range.
pub fn save(conn: &mut Connection, owner_key: &str, prefs: &Preferences) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO app_settings (owner_key, notifications_enabled, notification_sound, \
         start_minimized, game_detection_enabled, game_scan_interval_secs, input_device, \
         output_device, video_device_id, video_device_label, input_volume, output_volume, \
         noise_suppression, echo_cancellation, auto_away_minutes, \
         auto_volunteer_relay_for_new_friends, ringtone_enabled, ringtone_volume, \
         in_call_dnd_auto_enable) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19) \
         ON CONFLICT(owner_key) DO UPDATE SET \
         notifications_enabled = excluded.notifications_enabled, \
         notification_sound = excluded.notification_sound, \
         start_minimized = excluded.start_minimized, \
         game_detection_enabled = excluded.game_detection_enabled, \
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
         in_call_dnd_auto_enable = excluded.in_call_dnd_auto_enable",
        params![
            owner_key,
            prefs.notifications_enabled,
            prefs.notification_sound,
            prefs.start_minimized,
            prefs.game_detection_enabled,
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
}
