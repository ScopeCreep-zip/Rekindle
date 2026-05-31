use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::channels::CommunityEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

const REMINDER_LEAD_SECONDS: u64 = 10 * 60;
const IDLE_RECHECK_SECONDS: u64 = 5 * 60;

#[derive(Debug, Clone)]
struct PendingReminder {
    community_id: String,
    event_id: String,
    title: String,
    start_time: u64,
    fire_at: u64,
}

pub fn start_event_reminders(
    state: Arc<AppState>,
    pool: DbPool,
) -> tauri::async_runtime::JoinHandle<()> {
    let (wake_tx, mut wake_rx) = tokio::sync::watch::channel(0u64);
    *state.event_reminder_wake_tx.write() = Some(wake_tx);

    tauri::async_runtime::spawn(async move {
        let mut fired = HashSet::new();

        loop {
            let Some(reminder) = next_pending_reminder(&state, &pool, &fired).await else {
                let sleep = tokio::time::sleep(Duration::from_secs(IDLE_RECHECK_SECONDS));
                tokio::pin!(sleep);
                tokio::select! {
                    () = &mut sleep => {}
                    changed = wake_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
                continue;
            };

            let key = reminder_key(&reminder);
            let now = rekindle_utils::timestamp_secs();

            if reminder.fire_at <= now {
                if should_emit_reminder(&state, &pool, &reminder).await {
                    emit_reminder(&state, &reminder);
                }
                fired.insert(key);
                continue;
            }

            let wait_secs = reminder.fire_at.saturating_sub(now);
            let sleep = tokio::time::sleep(Duration::from_secs(wait_secs));
            tokio::pin!(sleep);
            tokio::select! {
                () = &mut sleep => {
                    if should_emit_reminder(&state, &pool, &reminder).await {
                        emit_reminder(&state, &reminder);
                    }
                    fired.insert(key);
                }
                changed = wake_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

pub fn wake_event_reminders(state: &Arc<AppState>) {
    if let Some(tx) = state.event_reminder_wake_tx.read().as_ref() {
        tx.send_modify(|value| *value = value.saturating_add(1));
    }
}

async fn next_pending_reminder(
    state: &Arc<AppState>,
    pool: &DbPool,
    fired: &HashSet<String>,
) -> Option<PendingReminder> {
    let owner_key = state_helpers::current_owner_key(state).ok()?;
    let now = rekindle_utils::timestamp_secs();

    let reminders = db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT community_id, id, title, start_time \
             FROM community_events \
             WHERE owner_key = ?1 AND status = 'scheduled' AND start_time > 0",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key], |row| {
                let start_time = row.get::<_, i64>(3).unwrap_or(0).unsigned_abs();
                Ok(PendingReminder {
                    community_id: row.get(0)?,
                    event_id: row.get(1)?,
                    title: row.get(2)?,
                    start_time,
                    fire_at: start_time.saturating_sub(REMINDER_LEAD_SECONDS),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .ok()?;

    reminders
        .into_iter()
        .filter(|reminder| reminder.start_time > now)
        .filter(|reminder| !fired.contains(&reminder_key(reminder)))
        .min_by_key(|reminder| reminder.fire_at.max(now))
}

async fn should_emit_reminder(
    state: &Arc<AppState>,
    pool: &DbPool,
    reminder: &PendingReminder,
) -> bool {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return false;
    };

    let community_id = reminder.community_id.clone();
    let event_id = reminder.event_id.clone();
    let start_time = reminder.start_time;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT 1 FROM community_events \
             WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3 \
             AND status = 'scheduled' AND start_time = ?4",
        )?;
        let exists = stmt.exists(rusqlite::params![
            owner_key,
            community_id,
            event_id,
            start_time.cast_signed(),
        ])?;
        Ok(exists)
    })
    .await
    .unwrap_or(false)
}

fn emit_reminder(state: &Arc<AppState>, reminder: &PendingReminder) {
    let Some(app_handle) = state_helpers::app_handle(state) else {
        return;
    };

    crate::event_dispatch::emit_live(
        &app_handle,
        "community-event",
        &CommunityEvent::EventReminder {
            community_id: reminder.community_id.clone(),
            event_id: reminder.event_id.clone(),
            title: reminder.title.clone(),
            minutes_until_start: u32::try_from(REMINDER_LEAD_SECONDS / 60).unwrap_or(10),
        },
    );
}

fn reminder_key(reminder: &PendingReminder) -> String {
    format!(
        "{}:{}:{}",
        reminder.community_id, reminder.event_id, reminder.start_time
    )
}
