# Plan: Refactoring Item #7 — Message Persistence Consolidation

## Problem Statement

The `INSERT INTO messages` SQL statement is duplicated across **5 locations** in 4 files, each with slight variations in column sets, `is_read` values, and `mek_generation` inclusion. The persist+emit pattern (insert row then emit `ChatEvent::MessageReceived`) is also duplicated across 4 of those sites. This creates maintenance risk — any schema change to the `messages` table requires updating 5 SQL strings.

## Current State — 5 INSERT Sites

| # | File | Function | Direction | Type | is_read | mek_gen | Emit | DB Method |
|---|------|----------|-----------|------|---------|---------|------|-----------|
| 1 | `commands/chat.rs:41-49` | `send_message` | Outgoing | dm | 1 | No | MessageAck | `db_call` |
| 2 | `commands/community.rs:503-511` | `send_channel_message` | Outgoing | channel | 1 | Yes | MessageReceived (line 551) | `db_call` |
| 3 | `services/message_service.rs:235-242` | `handle_direct_message` | Incoming | dm | 0 | No | MessageReceived (line 253) | `db_fire` |
| 4 | `services/message_service.rs:276-283` | `handle_channel_message` | Incoming | channel | 0 | No | MessageReceived (line 285) | `db_fire` |
| 5 | `services/veilid_service.rs:321-328` | `handle_broadcast_new_message` | Incoming | channel | 0 | Yes | MessageReceived (line 331) | `db_fire` |

### Key Variations
- **`is_read`**: 1 for outgoing (own messages), 0 for incoming
- **`mek_generation`**: only present for channel messages in communities (sites 2, 5); absent for DMs and legacy plaintext channels
- **`db_call` vs `db_fire`**: outgoing messages use `db_call` (awaited — failure blocks send); incoming use `db_fire` (fire-and-forget — failure logged but doesn't block event emission)
- **Emit type**: outgoing DM emits `MessageAck`; outgoing channel and all incoming emit `MessageReceived`
- **Unread count**: only site 3 (incoming DM) increments `unread_count` in state

## Proposed Solution

### New file: `src-tauri/src/message_repo.rs`

A thin module with **pure rusqlite functions** (no async, no pool, no state — just `&Connection` in, `Result` out). This follows the same pattern as `db_helpers.rs` but for message-specific SQL. Each call site keeps control over `db_call` vs `db_fire`, and over what events to emit.

```rust
//! Message persistence helpers.
//!
//! Pure `rusqlite` functions that encapsulate the SQL for the `messages` table.
//! Callers wrap these in `db_call` or `db_fire` as appropriate.

/// Insert a direct message into the messages table.
pub fn insert_dm(
    conn: &rusqlite::Connection,
    owner_key: &str,
    peer_key: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, \
         sender_key, body, timestamp, is_read) \
         VALUES (?, ?, 'dm', ?, ?, ?, ?)",
        rusqlite::params![owner_key, peer_key, sender_key, body, timestamp, is_read as i32],
    )?;
    Ok(())
}

/// Insert a channel message into the messages table.
pub fn insert_channel_message(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    sender_key: &str,
    body: &str,
    timestamp: i64,
    is_read: bool,
    mek_generation: Option<i64>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, \
         sender_key, body, timestamp, is_read, mek_generation) \
         VALUES (?, ?, 'channel', ?, ?, ?, ?, ?)",
        rusqlite::params![
            owner_key, channel_id, sender_key, body, timestamp,
            is_read as i32, mek_generation
        ],
    )?;
    Ok(())
}
```

**Why two functions instead of one?** The `conversation_type` is a CHECK constraint (`'dm'` or `'channel'`), and DMs never have `mek_generation`. Keeping them separate makes each call site's intent explicit and prevents accidentally writing a DM with an MEK generation or a channel message without one. It also avoids a `ConversationType` enum parameter that would just add indirection.

**Why NOT a "persist + emit" combo helper?** The emit patterns differ too much:
- Site 1 emits `MessageAck` (not `MessageReceived`)
- Site 3 also updates `unread_count` between persist and emit
- Sites 2 and 5 get their `from`/`conversation_id` from different sources
- Sites 1-2 use `db_call` (must await success before proceeding); sites 3-5 use `db_fire` (emit immediately)

Forcing these into one helper would require enough parameters/flags to be worse than the current code. The SQL dedup alone is the right level of abstraction.

### Changes Per File

#### 1. `src-tauri/src/message_repo.rs` (NEW)
- Create with `insert_dm()` and `insert_channel_message()` as described above.

#### 2. `src-tauri/src/lib.rs`
- Add `pub mod message_repo;` to module declarations.

#### 3. `src-tauri/src/commands/chat.rs` (site 1)
```rust
// BEFORE (lines 41-49):
let ok = owner_key.clone();
db_call(pool.inner(), move |conn| {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read) \
         VALUES (?, ?, 'dm', ?, ?, ?, 1)",
        rusqlite::params![ok, to_clone, sender_key_clone, body_clone, timestamp],
    )?;
    Ok(())
})
.await?;

// AFTER:
let ok = owner_key.clone();
db_call(pool.inner(), move |conn| {
    crate::message_repo::insert_dm(conn, &ok, &to_clone, &sender_key_clone, &body_clone, timestamp, true)
})
.await?;
```

#### 4. `src-tauri/src/commands/community.rs` (site 2)
```rust
// BEFORE (lines 503-511):
let ok = owner_key;
db_call(pool.inner(), move |conn| {
    conn.execute(
        "INSERT INTO messages (..., mek_generation) VALUES (?, ?, 'channel', ?, ?, ?, 1, ?)",
        rusqlite::params![ok, channel_id_clone, sender_key_clone, body_clone, timestamp, mek_generation.cast_signed()],
    )?;
    Ok(())
})
.await?;

// AFTER:
let ok = owner_key;
let mg = mek_generation.cast_signed();
db_call(pool.inner(), move |conn| {
    crate::message_repo::insert_channel_message(conn, &ok, &channel_id_clone, &sender_key_clone, &body_clone, timestamp, true, Some(mg))
})
.await?;
```

#### 5. `src-tauri/src/services/message_service.rs` — `handle_direct_message` (site 3)
```rust
// BEFORE (lines 232-242):
let owner_key = state_helpers::owner_key_or_default(state);
let sender = sender_hex.to_string();
let body_clone = body.to_string();
db_fire(pool, "persist incoming message", move |conn| {
    conn.execute(
        "INSERT INTO messages (...) VALUES (?, ?, 'dm', ?, ?, ?, 0)",
        rusqlite::params![owner_key, sender, sender, body_clone, timestamp],
    )?;
    Ok(())
});

// AFTER:
let owner_key = state_helpers::owner_key_or_default(state);
let sender = sender_hex.to_string();
let body_clone = body.to_string();
db_fire(pool, "persist incoming message", move |conn| {
    crate::message_repo::insert_dm(conn, &owner_key, &sender, &sender, &body_clone, timestamp, false)
});
```

#### 6. `src-tauri/src/services/message_service.rs` — `handle_channel_message` (site 4)
```rust
// BEFORE (lines 272-283):
let owner_key = state_helpers::owner_key_or_default(state);
let sender = sender_hex.to_string();
let ch_id = channel_id.to_string();
let body_clone = body.to_string();
db_fire(pool, "persist channel message", move |conn| {
    conn.execute(
        "INSERT INTO messages (...) VALUES (?, ?, 'channel', ?, ?, ?, 0)",
        rusqlite::params![owner_key, ch_id, sender, body_clone, timestamp],
    )?;
    Ok(())
});

// AFTER:
let owner_key = state_helpers::owner_key_or_default(state);
let sender = sender_hex.to_string();
let ch_id = channel_id.to_string();
let body_clone = body.to_string();
db_fire(pool, "persist channel message", move |conn| {
    crate::message_repo::insert_channel_message(conn, &owner_key, &ch_id, &sender, &body_clone, timestamp, false, None)
});
```

#### 7. `src-tauri/src/services/veilid_service.rs` — `handle_broadcast_new_message` (site 5)
```rust
// BEFORE (lines 313-328):
let owner_key = state_helpers::owner_key_or_default(state);
let pool: tauri::State<'_, DbPool> = app_handle.state();
let cid = msg.channel_id.clone();
let spn = msg.sender_pseudonym.clone();
let body_text = body.clone();
let ts = msg.timestamp.cast_signed();
let mg = msg.mek_generation.cast_signed();
db_fire(pool.inner(), "store community message", move |conn| {
    conn.execute(
        "INSERT INTO messages (..., mek_generation) VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
        rusqlite::params![owner_key, cid, spn, body_text, ts, mg],
    )?;
    Ok(())
});

// AFTER:
let owner_key = state_helpers::owner_key_or_default(state);
let pool: tauri::State<'_, DbPool> = app_handle.state();
let cid = msg.channel_id.clone();
let spn = msg.sender_pseudonym.clone();
let body_text = body.clone();
let ts = msg.timestamp.cast_signed();
let mg = msg.mek_generation.cast_signed();
db_fire(pool.inner(), "store community message", move |conn| {
    crate::message_repo::insert_channel_message(conn, &owner_key, &cid, &spn, &body_text, ts, false, Some(mg))
});
```

## What This Does NOT Change

- **No new abstractions or traits** — just two plain functions
- **No changes to `db_call` vs `db_fire` decisions** — callers keep their existing error handling
- **No changes to event emission** — each site still emits its own events with its own parameters
- **No changes to unread count logic** — `handle_direct_message` still updates `unread_count` inline
- **No behavioral changes** — identical SQL, identical column values, identical ordering

## Impact

- **5 inline SQL strings → 2 functions** (one for DMs, one for channels)
- **~30 lines of duplicated SQL → 0** — all INSERT INTO messages SQL lives in one file
- **Future schema changes** to the `messages` table only need to update `message_repo.rs`
- **No clippy flags needed** — functions are small and focused
- **Total new code**: ~30 lines in `message_repo.rs` + 1 line in `lib.rs`
- **Total removed code**: ~25 lines of inline SQL across 4 files (net: ~+6 lines, but all duplication eliminated)

## Verification

1. `cargo clippy --workspace -- -D warnings` passes
2. `cargo test --workspace` passes
3. All 5 call sites produce identical SQL to before (same columns, same values, same parameter order)
4. `pnpm tauri dev` — send DM, receive DM, send channel message, receive channel message — all persist correctly
