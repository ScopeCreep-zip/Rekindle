use std::sync::Arc;

use crate::server_state::ServerState;

/// Periodically clean up stale rate limiter entries to prevent unbounded memory growth.
pub fn cleanup_rate_limiter(state: &Arc<ServerState>) {
    let now = rekindle_utils::timestamp_secs_i64();
    state.rate_limiter.cleanup(now);
}

/// Remove stale slowmode tracker entries (older than 1 hour).
pub fn cleanup_slowmode_tracker(state: &Arc<ServerState>) {
    let now = rekindle_utils::timestamp_secs_i64();
    let cutoff = now - 3600;
    let mut tracker = state.slowmode_last_message.write();
    tracker.retain(|_, ts| *ts > cutoff);
}

/// Archive threads where (now - last_message_at) > auto_archive_seconds.
pub fn auto_archive_stale_threads(state: &Arc<ServerState>) {
    let now = rekindle_utils::timestamp_secs_i64();
    crate::db_helpers::db_fire(&state.db, "auto-archive threads", |conn| {
        conn.execute(
            "UPDATE server_threads SET archived = 1 \
             WHERE archived = 0 AND auto_archive_seconds > 0 \
             AND (? - last_message_at) > auto_archive_seconds",
            rusqlite::params![now],
        )?;
        Ok(())
    });
}

/// Transition event lifecycle states:
/// - "scheduled" → "active" when start_time has passed
/// - "active" → "completed" when end_time has passed
pub fn advance_event_lifecycle(state: &Arc<ServerState>) {
    let now = rekindle_utils::timestamp_secs_i64();
    crate::db_helpers::db_fire(&state.db, "advance event lifecycle", |conn| {
        // scheduled → active: start_time has passed
        conn.execute(
            "UPDATE server_events SET status = 'active' \
             WHERE status = 'scheduled' AND start_time <= ?",
            rusqlite::params![now],
        )?;
        // active → completed: end_time has passed (only events with an end_time)
        conn.execute(
            "UPDATE server_events SET status = 'completed' \
             WHERE status = 'active' AND end_time IS NOT NULL AND end_time <= ?",
            rusqlite::params![now],
        )?;
        Ok(())
    });
}

/// Delete events that have been completed or canceled for more than 30 days.
pub fn cleanup_past_events(state: &Arc<ServerState>) {
    let now = rekindle_utils::timestamp_secs_i64();
    let cutoff = now - 30 * 86400;
    crate::db_helpers::db_fire(&state.db, "cleanup past events", |conn| {
        // Delete RSVPs for old events first (FK not enforced for these)
        conn.execute(
            "DELETE FROM server_event_rsvps WHERE event_id IN \
             (SELECT id FROM server_events WHERE status IN ('completed','canceled') AND created_at < ?)",
            rusqlite::params![cutoff],
        )?;
        conn.execute(
            "DELETE FROM server_events WHERE status IN ('completed','canceled') AND created_at < ?",
            rusqlite::params![cutoff],
        )?;
        Ok(())
    });
}

/// Check for events starting within 15 minutes and broadcast reminders.
pub fn check_event_reminders(state: &Arc<ServerState>) -> Vec<(String, String, String, u32)> {
    let now = rekindle_utils::timestamp_secs_i64();
    let window_end = now + 15 * 60;

    let events = crate::db_helpers::db_call_or_default(&state.db, |conn| {
        let mut stmt = conn.prepare(
            "SELECT community_id, id, title, start_time \
             FROM server_events WHERE reminder_sent = 0 \
             AND start_time > ? AND start_time <= ?",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![now, window_end], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        Ok(rows)
    });

    let mut reminders = Vec::new();
    let mut event_ids = Vec::new();
    for (community_id, event_id, title, start_time) in &events {
        let minutes = u32::try_from(((start_time - now) / 60).max(0)).unwrap_or(0);
        reminders.push((community_id.clone(), event_id.clone(), title.clone(), minutes));
        event_ids.push(event_id.clone());
    }

    // Batch-mark all events as reminded in a single DB call
    if !event_ids.is_empty() {
        crate::db_helpers::db_fire(&state.db, "mark events reminded", |conn| {
            let placeholders: Vec<&str> = event_ids.iter().map(|_| "?").collect();
            let sql = format!(
                "UPDATE server_events SET reminder_sent = 1 WHERE id IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                event_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
            conn.execute(&sql, params.as_slice())?;
            Ok(())
        });
    }

    reminders
}
