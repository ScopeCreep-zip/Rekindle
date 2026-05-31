//! Phase 23.C — event-resume orchestration lifted from
//! `commands/event.rs`. Replays journal entries with
//! `cursor > effective_cursor` (where `effective_cursor =
//! max(replay_watermark, last_cursor)`) through the live dispatch
//! queue so listeners cannot tell replay from live. Watermark ensures
//! each entry broadcasts AT MOST ONCE across concurrent calls.

use crate::state::SharedState;

pub fn event_resume_inner(state: &SharedState, last_cursor: u64) -> u64 {
    let backlog = {
        let mut watermark = state.event_replay_watermark.lock();
        let effective = (*watermark).max(last_cursor);
        let snapshot = state.event_journal.replay_since(effective);
        if let Some(last_entry) = snapshot.last() {
            *watermark = last_entry.cursor;
        }
        snapshot
    };
    let count = u64::try_from(backlog.len()).unwrap_or(u64::MAX);
    for entry in backlog {
        crate::event_dispatch::emit_value(state, &entry.event.channel, &entry.event.payload);
        crate::event_dispatch::emit_now(
            state,
            "cursor-tick",
            &crate::event_dispatch::CursorTick {
                cursor: entry.cursor,
            },
        );
    }
    count
}
