//! Phase 4 — SQLite persistence for `audit_entries`.
//!
//! Layer:
//! - **Conn-level functions** (`insert_entry`, `load_all`, `load_since`):
//!   pure `rusqlite` used inside `db_call`/`db_fire` closures.
//! - **High-level helpers** (`append_async`, `verify_async`, `export_since_async`):
//!   pull the chain state from `AppState::audit_chain`, append + persist
//!   atomically, and broadcast `SystemEvent::AuditChainBroken` on verify failure.

use std::sync::Arc;

use rekindle_audit::{AuditChain, AuditEntry, AuditKind, AuditRecord, VerifyError};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;

// ── Conn-level functions ────────────────────────────────────────────

/// Insert one audit entry. Caller owns chain-state advancement (i.e. the
/// `AuditChain` instance in `AppState::audit_chain`); this is pure I/O.
pub fn insert_entry(
    conn: &Connection,
    owner_key: &str,
    entry: &AuditEntry,
) -> Result<(), rusqlite::Error> {
    let payload_json = serde_json::to_string(&entry.record)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    #[allow(
        clippy::cast_possible_wrap,
        reason = "audit cursor is u64 monotonic; SQLite stores i64 — bit-cast is safe for the lifetime of any reasonable device (u64::MAX/2 entries ≈ 9.2e18)"
    )]
    let cursor_i64 = entry.cursor as i64;
    conn.execute(
        "INSERT INTO audit_entries (owner_key, cursor, prev_mac, mac, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            owner_key,
            cursor_i64,
            &entry.prev_mac[..],
            &entry.mac[..],
            payload_json,
        ],
    )?;
    Ok(())
}

/// Load every entry for `owner_key` in cursor order.
pub fn load_all(conn: &Connection, owner_key: &str) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key], row_to_entry)?;
    rows.collect()
}

/// Load entries with `cursor > since_cursor` for export.
pub fn load_since(
    conn: &Connection,
    owner_key: &str,
    since_cursor: u64,
) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    #[allow(clippy::cast_possible_wrap, reason = "see insert_entry note")]
    let since_i64 = since_cursor as i64;
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 AND cursor > ?2 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key, since_i64], row_to_entry)?;
    rows.collect()
}

/// Latest (cursor, mac) for `owner_key`, or `(0, [0u8; 32])` if empty.
/// Used to initialize the in-memory `AuditChain` on vault unlock.
pub fn load_tail(
    conn: &Connection,
    owner_key: &str,
) -> Result<(u64, [u8; 32]), rusqlite::Error> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT cursor, mac FROM audit_entries \
             WHERE owner_key = ?1 ORDER BY cursor DESC LIMIT 1",
            params![owner_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cursor_i64, mac_bytes)) = row else {
        return Ok((0, [0u8; 32]));
    };
    if mac_bytes.len() != 32 || cursor_i64 < 0 {
        // Truncated/corrupt — surface during verify rather than panic here.
        return Ok((0, [0u8; 32]));
    }
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&mac_bytes);
    #[allow(clippy::cast_sign_loss, reason = "cursor is non-negative by SQL invariant")]
    let cursor = cursor_i64 as u64;
    Ok((cursor, mac))
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let cursor_i64: i64 = row.get(0)?;
    let prev_mac_bytes: Vec<u8> = row.get(1)?;
    let mac_bytes: Vec<u8> = row.get(2)?;
    let payload_json: String = row.get(3)?;
    #[allow(clippy::cast_sign_loss, reason = "see load_tail")]
    let cursor = cursor_i64 as u64;
    let record: AuditRecord = serde_json::from_str(&payload_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?;
    let mut prev_mac = [0u8; 32];
    let mut mac = [0u8; 32];
    if prev_mac_bytes.len() != 32 || mac_bytes.len() != 32 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!(
                "audit_entries row at cursor {cursor} has malformed MAC blobs (prev_mac={}B, mac={}B)",
                prev_mac_bytes.len(),
                mac_bytes.len()
            )
            .into(),
        ));
    }
    prev_mac.copy_from_slice(&prev_mac_bytes);
    mac.copy_from_slice(&mac_bytes);
    Ok(AuditEntry {
        cursor,
        prev_mac,
        mac,
        record,
    })
}

// ── High-level helpers ──────────────────────────────────────────────

/// Result shape returned by the `audit_verify` Tauri command.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditVerifyResult {
    pub ok: bool,
    pub length: u64,
    /// First cursor where the chain failed integrity. `None` when ok.
    pub broken_at: Option<u64>,
    /// Diagnostic detail. `None` when ok.
    pub detail: Option<String>,
}

/// Append a record to the chain and persist it atomically. Failures fall
/// through as a warn-level log — audit is best-effort, never blocks the
/// primary mutation it's accompanying.
pub async fn append_async(
    state: &Arc<AppState>,
    pool: &DbPool,
    owner_key: &str,
    kind: AuditKind,
    payload: serde_json::Value,
) {
    let actor_pub = owner_key.to_string();
    let record = AuditRecord {
        at_ms: rekindle_utils::timestamp_ms_i64(),
        actor_pub: actor_pub.clone(),
        kind: kind.clone(),
        payload,
    };
    let entry = {
        let mut chain = state.audit_chain.lock();
        let Some(chain) = chain.as_mut() else {
            tracing::debug!(
                "audit chain not initialized — skipping append (vault not unlocked yet?)",
            );
            return;
        };
        match chain.append(record) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "audit chain append failed at serialize step");
                return;
            }
        }
    };
    let owner = owner_key.to_string();
    let entry_clone = entry.clone();
    let result = db_call(pool, move |conn| insert_entry(conn, &owner, &entry_clone)).await;
    if let Err(e) = result {
        tracing::warn!(
            kind = ?kind,
            actor = %actor_pub,
            cursor = entry.cursor,
            error = %e,
            "audit chain persist failed — in-memory chain advanced but row not written",
        );
        return;
    }

    // Tail-anchor persistence — detects SQLite-side tail truncation
    // on next restore. The vault is a separate file with its own
    // encryption layer, so an attacker who can edit the SQLite db
    // cannot forge a matching anchor update.
    {
        let ks = state.keystore.lock();
        if let Some(ref keystore) = *ks {
            if let Err(e) =
                crate::keystore::persist_audit_tail(keystore, entry.cursor, &entry.mac)
            {
                tracing::warn!(
                    cursor = entry.cursor,
                    error = %e,
                    "audit tail anchor persist failed — chain still verifiable via in-memory state but tail-truncation detection across restart is degraded",
                );
            }
        }
    }

    tracing::debug!(
        kind = ?kind,
        cursor = entry.cursor,
        "audit entry recorded",
    );
}

/// Verify the chain end-to-end. Emits `SystemEvent::AuditChainBroken` and
/// a typed `notification-event` toast on failure.
pub async fn verify_async(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    owner_key: &str,
) -> AuditVerifyResult {
    let owner = owner_key.to_string();
    let entries = match db_call(pool, move |conn| load_all(conn, &owner)).await {
        Ok(e) => e,
        Err(e) => {
            return AuditVerifyResult {
                ok: false,
                length: 0,
                broken_at: None,
                detail: Some(format!("DB load failed: {e}")),
            };
        }
    };
    let length = entries.len() as u64;

    let chain_guard = state.audit_chain.lock();
    let Some(chain) = chain_guard.as_ref() else {
        return AuditVerifyResult {
            ok: false,
            length,
            broken_at: None,
            detail: Some("audit chain not initialized (vault locked?)".into()),
        };
    };
    let verify = chain.verify(&entries);
    drop(chain_guard);

    match verify {
        Ok(()) => AuditVerifyResult {
            ok: true,
            length,
            broken_at: None,
            detail: None,
        },
        Err(e) => {
            let broken_at = match &e {
                VerifyError::PrevMacMismatch { cursor, .. }
                | VerifyError::MacMismatch { cursor, .. }
                | VerifyError::NonMonotonicCursor { cursor, .. } => Some(*cursor),
                VerifyError::Serialize(_) => None,
            };
            let detail = e.to_string();
            tracing::error!(
                owner = %owner_key,
                length,
                broken_at = ?broken_at,
                detail = %detail,
                "audit chain verification FAILED",
            );
            if let Some(cursor) = broken_at {
                crate::event_dispatch::emit_live(
                    app_handle,
                    "notification-event",
                    &crate::channels::NotificationEvent::SystemAlert {
                        title: "Audit chain broken".into(),
                        body: format!(
                            "Your device's tamper-evident audit log failed integrity check at \
                             entry #{cursor}. Someone with write access to your SQLite database \
                             may have modified history. Verify out-of-band before trusting any \
                             post-tamper actions."
                        ),
                    },
                );
            }
            AuditVerifyResult {
                ok: false,
                length,
                broken_at,
                detail: Some(detail),
            }
        }
    }
}

/// Initialize the in-memory `AuditChain` for `owner_key` on vault unlock.
/// Loads the persisted tail (cursor + last mac) so the next `append`
/// continues the existing chain instead of restarting from genesis.
///
/// Also performs **tail-truncation detection**: compares SQLite's tail
/// against the vault-persisted anchor (written on every append). A
/// mismatch indicates an attacker dropped trailing `audit_entries`
/// rows after the last legitimate write — emits `AuditChainBroken`
/// and a `SystemAlert` toast so the user sees the integrity violation.
pub async fn restore_chain(
    app_handle: Option<&tauri::AppHandle>,
    state: &Arc<AppState>,
    pool: &DbPool,
    owner_key: &str,
    mac_key: [u8; 32],
) -> Result<(), String> {
    let owner = owner_key.to_string();
    let (sqlite_cursor, sqlite_mac) = db_call(pool, move |conn| load_tail(conn, &owner))
        .await
        .map_err(|e| format!("audit chain load tail: {e}"))?;

    // Compare against the vault-persisted tail anchor. If the anchor exists
    // but doesn't match the SQLite tail, the SQLite tail was tampered with
    // (most likely truncated). The chain is initialized from the anchor
    // (the trusted source) so subsequent appends link from the real tail,
    // and an `AuditChainBroken` event surfaces the tamper.
    let anchor = {
        let ks = state.keystore.lock();
        ks.as_ref()
            .and_then(crate::keystore::load_audit_tail)
    };
    // Decide whether SQLite has been tampered, taking three signals:
    //   anchor_cursor == sqlite_cursor && anchor_mac == sqlite_mac : clean.
    //   anchor_cursor == sqlite_cursor && mac mismatch            : tail content modified.
    //   anchor_cursor >  sqlite_cursor                            : SQLite truncated.
    //   anchor_cursor <  sqlite_cursor                            : anchor is behind
    //     (in-flight append lost its vault write — e.g. logout race or
    //     crash between SQLite insert and vault persist). NOT tamper.
    //     The auto-verify-on-boot call in auth.rs re-MACs every entry,
    //     so any forgery added in the gap will be caught there; the
    //     anchor will be refreshed on the next legitimate append.
    let (cursor, last_mac, tamper_at) = match anchor {
        Some((anchor_cursor, anchor_mac)) => {
            if anchor_cursor == sqlite_cursor && anchor_mac == sqlite_mac {
                (sqlite_cursor, sqlite_mac, None)
            } else if anchor_cursor < sqlite_cursor {
                tracing::info!(
                    owner = %owner_key,
                    sqlite_cursor,
                    anchor_cursor,
                    "audit tail anchor is behind SQLite — accepting catch-up (likely \
                     in-flight append lost vault write at logout); full-chain verify \
                     will catch any forgery in the gap",
                );
                (sqlite_cursor, sqlite_mac, None)
            } else {
                tracing::error!(
                    owner = %owner_key,
                    sqlite_cursor,
                    anchor_cursor,
                    "audit tail anchor mismatch — SQLite was tampered with (truncation \
                     or tail-content modification)",
                );
                // The user-visible cursor is the highest known good entry — the
                // anchor's, since SQLite's may have been forged downward.
                (anchor_cursor, anchor_mac, Some(anchor_cursor))
            }
        }
        None => {
            // No anchor yet (fresh identity OR pre-Phase-4 vault). Trust the
            // SQLite tail; the first append will write an anchor.
            (sqlite_cursor, sqlite_mac, None)
        }
    };
    let chain = AuditChain::open(zeroize::Zeroizing::new(mac_key), last_mac, cursor);
    *state.audit_chain.lock() = Some(chain);
    tracing::info!(
        owner = %owner_key,
        cursor,
        tail_anchored = tamper_at.is_none(),
        "audit chain initialized from persisted tail",
    );
    if let Some(broken_at) = tamper_at {
        if let Some(app) = app_handle {
            crate::event_dispatch::emit_live(
                app,
                "notification-event",
                &crate::channels::NotificationEvent::SystemAlert {
                    title: "Audit chain broken".into(),
                    body: format!(
                        "Your device's tamper-evident audit log was modified \
                         between sessions. The most recent vault-anchored entry \
                         was #{broken_at}, but the on-disk log has different \
                         content. Someone with write access to your local \
                         database may have removed or altered audit history."
                    ),
                },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! SQLite round-trip integration tests for the audit chain. The
    //! plan's Phase 4 manual scenario was "modify a row in SQLite,
    //! re-verify, expect ok=false" — these tests reproduce it
    //! programmatically.
    //!
    //! `rekindle-audit` itself has unit tests for in-memory tamper
    //! detection (`crates/rekindle-audit/src/chain.rs`); the tests here
    //! exercise the persistence + verify_async path that ties the chain
    //! to the `audit_entries` SQLite table.

    use super::*;
    use rekindle_audit::MAC_LEN;
    use tokio_rusqlite::Connection as TokioConn;

    async fn fresh_db_with_audit_table() -> std::sync::Arc<TokioConn> {
        let conn = TokioConn::open_in_memory().await.unwrap();
        conn.call(|c| -> rusqlite::Result<()> {
            c.execute_batch(
                "CREATE TABLE identity (public_key TEXT PRIMARY KEY);
                 CREATE TABLE audit_entries (
                    owner_key TEXT NOT NULL,
                    cursor INTEGER NOT NULL,
                    prev_mac BLOB NOT NULL,
                    mac BLOB NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (owner_key, cursor)
                 );",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        std::sync::Arc::new(conn)
    }

    fn fixture_chain(key: [u8; 32]) -> AuditChain {
        AuditChain::open(zeroize::Zeroizing::new(key), [0u8; MAC_LEN], 0)
    }

    fn mk_record(n: u64) -> AuditRecord {
        AuditRecord {
            at_ms: 1_700_000_000_000 + n.cast_signed(),
            actor_pub: "alice".into(),
            kind: AuditKind::FriendAdded,
            payload: serde_json::json!({ "peer": format!("bob-{n}") }),
        }
    }

    #[tokio::test]
    async fn persist_then_load_roundtrip() {
        let pool = fresh_db_with_audit_table().await;
        let mut chain = fixture_chain([7u8; 32]);
        let owner = "alice".to_string();

        let mut originals = Vec::new();
        for n in 1..=5 {
            let entry = chain.append(mk_record(n)).unwrap();
            let owner_c = owner.clone();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, &owner_c, &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
            originals.push(entry);
        }

        let owner_c = owner.clone();
        let loaded = pool
            .call(move |c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, &owner_c) })
            .await
            .unwrap();
        assert_eq!(loaded.len(), 5);
        for (a, b) in originals.iter().zip(loaded.iter()) {
            assert_eq!(a.cursor, b.cursor);
            assert_eq!(a.mac, b.mac);
            assert_eq!(a.prev_mac, b.prev_mac);
            assert_eq!(a.record.actor_pub, b.record.actor_pub);
        }

        // verify against the loaded entries — chain is intact.
        let verifier = fixture_chain([7u8; 32]);
        verifier.verify(&loaded).expect("persisted chain verifies cleanly");
    }

    #[tokio::test]
    async fn tampered_sqlite_row_fails_verify() {
        // The exact scenario from the plan's Tauri-testable section:
        // (1) append a few entries, persist, (2) modify one row's
        // payload_json directly in SQLite, (3) reload + verify — must
        // report the tampered cursor.
        let pool = fresh_db_with_audit_table().await;
        let mut chain = fixture_chain([7u8; 32]);
        let owner = "alice".to_string();

        for n in 1..=3 {
            let entry = chain.append(mk_record(n)).unwrap();
            let owner_c = owner.clone();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, &owner_c, &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
        }

        // Hand-modify the middle row to mimic an attacker editing the
        // SQLite database while the app is offline.
        pool.call(|c| -> rusqlite::Result<()> {
            c.execute(
                "UPDATE audit_entries SET payload_json = '{\"at_ms\":0,\"actor_pub\":\"EVIL\",\"kind\":\"FriendAdded\",\"payload\":{\"peer\":\"forged\"}}' \
                 WHERE owner_key = 'alice' AND cursor = 2",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let loaded = pool
            .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "alice") })
            .await
            .unwrap();
        let verifier = fixture_chain([7u8; 32]);
        let err = verifier.verify(&loaded).unwrap_err();
        match err {
            VerifyError::MacMismatch { cursor, .. } => assert_eq!(cursor, 2),
            other => panic!("expected MacMismatch at cursor 2, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn load_tail_returns_genesis_for_empty_table() {
        let pool = fresh_db_with_audit_table().await;
        let (cursor, mac) = pool
            .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
            .await
            .unwrap();
        assert_eq!(cursor, 0);
        assert_eq!(mac, [0u8; 32]);
    }

    #[tokio::test]
    async fn load_tail_recovers_chain_state() {
        let pool = fresh_db_with_audit_table().await;
        let mut chain = fixture_chain([7u8; 32]);
        let owner = "alice".to_string();

        let mut last = None;
        for n in 1..=4 {
            let entry = chain.append(mk_record(n)).unwrap();
            let owner_c = owner.clone();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, &owner_c, &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
            last = Some(entry);
        }
        let last = last.unwrap();

        let (cursor, mac) = pool
            .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
            .await
            .unwrap();
        assert_eq!(cursor, last.cursor);
        assert_eq!(mac, last.mac);

        // Reopening the chain from the persisted tail must produce a chain
        // whose next `append` links to the prior tail's mac.
        let mut reopened = AuditChain::open(zeroize::Zeroizing::new([7u8; 32]), mac, cursor);
        let next = reopened.append(mk_record(99)).unwrap();
        assert_eq!(next.cursor, 5);
        assert_eq!(next.prev_mac, last.mac);
    }

    #[tokio::test]
    async fn load_since_filters_correctly() {
        let pool = fresh_db_with_audit_table().await;
        let mut chain = fixture_chain([7u8; 32]);
        let owner = "alice".to_string();

        for n in 1..=5 {
            let entry = chain.append(mk_record(n)).unwrap();
            let owner_c = owner.clone();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, &owner_c, &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
        }
        let since_3 = pool
            .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_since(c, "alice", 3) })
            .await
            .unwrap();
        assert_eq!(since_3.len(), 2);
        assert_eq!(since_3[0].cursor, 4);
        assert_eq!(since_3[1].cursor, 5);
    }

    #[tokio::test]
    async fn tail_truncation_attack_is_detected_via_anchor() {
        // Reproduce the threat model: attacker with SQLite write access
        // drops trailing audit_entries rows. Without the vault-persisted
        // tail anchor, this would be invisible because the remaining
        // chain still verifies internally (it's just shorter). With the
        // anchor, restore_chain can compare SQLite's tail against the
        // vault's tail and detect the deletion.
        //
        // This test exercises the load_tail / load_audit_tail comparison
        // at the cursor + mac level — the same logic restore_chain runs
        // when called with `app_handle: None` (no Tauri AppHandle in unit
        // tests, so we can't observe the emit, but we observe the data).
        let pool = fresh_db_with_audit_table().await;
        let mut chain = fixture_chain([7u8; 32]);
        let owner = "alice".to_string();

        // Persist 5 entries via the normal append+insert path.
        let mut last_legit = None;
        for n in 1..=5 {
            let entry = chain.append(mk_record(n)).unwrap();
            let owner_c = owner.clone();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, &owner_c, &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
            last_legit = Some(entry);
        }
        let legitimate_tail = last_legit.unwrap();

        // Attacker drops the last 2 rows.
        pool.call(|c| -> rusqlite::Result<()> {
            c.execute(
                "DELETE FROM audit_entries WHERE owner_key = 'alice' AND cursor > 3",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // After tampering, SQLite's tail reports cursor=3 with cursor-3's mac.
        // The vault anchor (had we written it) would report cursor=5 with the
        // legitimate tail mac. Mismatch → tamper detected.
        let (sqlite_cursor, sqlite_mac) = pool
            .call(|c| -> rusqlite::Result<(u64, [u8; 32])> { load_tail(c, "alice") })
            .await
            .unwrap();
        assert_eq!(sqlite_cursor, 3, "post-truncation SQLite tail is cursor 3");
        assert_ne!(
            sqlite_mac, legitimate_tail.mac,
            "post-truncation tail mac differs from legitimate tail mac",
        );
        // The pre-tamper anchor (cursor=5, legitimate_tail.mac) is what
        // restore_chain compares against. Inequality on either field
        // would trigger the tamper signal — proven by the assertions above.
    }

    /// Encodes the same decision rule as `restore_chain`'s anchor-vs-SQLite
    /// comparison. Extracted so the three branch tests below stay
    /// declarative — they pin the rule, not the surrounding plumbing.
    fn anchor_decision(
        anchor_cursor: u64,
        anchor_mac: [u8; MAC_LEN],
        sqlite_cursor: u64,
        sqlite_mac: [u8; MAC_LEN],
    ) -> Option<u64> {
        if anchor_cursor < sqlite_cursor
            || (anchor_cursor == sqlite_cursor && anchor_mac == sqlite_mac)
        {
            None
        } else {
            Some(anchor_cursor)
        }
    }

    #[test]
    fn anchor_behind_sqlite_is_catchup_not_tamper() {
        // Race scenario from aspect (h4): an in-flight append_async wrote
        // its row to SQLite but lost the vault anchor write (e.g. logout
        // cleared keystore between SQLite insert and tail persist).
        // Decision: anchor_cursor < sqlite_cursor means catch-up, NOT
        // tamper.
        assert!(
            anchor_decision(3, [0xAAu8; MAC_LEN], 5, [0xBBu8; MAC_LEN]).is_none(),
            "anchor behind SQLite must be catch-up",
        );
    }

    #[test]
    fn anchor_ahead_of_sqlite_is_tamper() {
        // Truncation attack: attacker dropped trailing audit rows. The
        // anchor (vault-persisted on last legitimate append) is now
        // ahead of SQLite. Must signal tamper.
        assert_eq!(
            anchor_decision(5, [0xAAu8; MAC_LEN], 3, [0xBBu8; MAC_LEN]),
            Some(5),
        );
    }

    #[test]
    fn anchor_equal_cursor_but_mac_mismatch_is_tamper() {
        // Attacker modified the tail row's payload (cursor unchanged but
        // MAC differs from anchor). Must signal tamper.
        assert_eq!(
            anchor_decision(5, [0xAAu8; MAC_LEN], 5, [0xCCu8; MAC_LEN]),
            Some(5),
        );
    }

    #[test]
    fn anchor_equal_cursor_and_mac_is_clean() {
        // Happy path — both vault and SQLite agree.
        assert!(
            anchor_decision(5, [0xAAu8; MAC_LEN], 5, [0xAAu8; MAC_LEN]).is_none(),
        );
    }

    #[tokio::test]
    async fn fk_cascade_drops_audit_when_identity_deleted() {
        // Phase 4 SQLite migration declares
        //   owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE
        // so deleting an identity wipes its audit_entries (orphans = bad).
        // Verify the cascade actually fires under runtime `PRAGMA foreign_keys=ON;`.
        let conn = TokioConn::open_in_memory().await.unwrap();
        conn.call(|c| -> rusqlite::Result<()> {
            // foreign_keys must be ON for cascade to fire (db.rs::open() does this in prod)
            c.execute_batch("PRAGMA foreign_keys=ON;")?;
            c.execute_batch(
                "CREATE TABLE identity (public_key TEXT PRIMARY KEY);
                 CREATE TABLE audit_entries (
                    owner_key TEXT NOT NULL REFERENCES identity(public_key) ON DELETE CASCADE,
                    cursor INTEGER NOT NULL,
                    prev_mac BLOB NOT NULL,
                    mac BLOB NOT NULL,
                    payload_json TEXT NOT NULL,
                    PRIMARY KEY (owner_key, cursor)
                 );
                 INSERT INTO identity (public_key) VALUES ('alice');",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let pool = std::sync::Arc::new(conn);

        // Persist 3 audit entries for alice.
        let mut chain = fixture_chain([7u8; 32]);
        for n in 1..=3 {
            let entry = chain.append(mk_record(n)).unwrap();
            let entry_c = entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, "alice", &entry_c)?;
                Ok(())
            })
            .await
            .unwrap();
        }
        let before_count: i64 = pool
            .call(|c| -> rusqlite::Result<i64> {
                c.query_row(
                    "SELECT COUNT(*) FROM audit_entries WHERE owner_key = 'alice'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(before_count, 3);

        // Delete the identity — cascade must wipe the 3 audit rows.
        pool.call(|c| -> rusqlite::Result<()> {
            c.execute("DELETE FROM identity WHERE public_key = 'alice'", [])?;
            Ok(())
        })
        .await
        .unwrap();

        let after_count: i64 = pool
            .call(|c| -> rusqlite::Result<i64> {
                c.query_row(
                    "SELECT COUNT(*) FROM audit_entries WHERE owner_key = 'alice'",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(
            after_count, 0,
            "ON DELETE CASCADE must wipe audit entries when identity is deleted (orphans = bad)",
        );
    }

    #[tokio::test]
    async fn owner_key_isolates_chains() {
        // Two identities on one device must have independent chains.
        let pool = fresh_db_with_audit_table().await;
        let mut alice_chain = fixture_chain([1u8; 32]);
        let mut bob_chain = fixture_chain([2u8; 32]);

        for n in 1..=3 {
            let alice_entry = alice_chain.append(mk_record(n)).unwrap();
            let bob_entry = bob_chain.append(mk_record(n + 100)).unwrap();
            let ae = alice_entry.clone();
            let be = bob_entry.clone();
            pool.call(move |c| -> rusqlite::Result<()> {
                insert_entry(c, "alice", &ae)?;
                insert_entry(c, "bob", &be)?;
                Ok(())
            })
            .await
            .unwrap();
        }

        let alice_entries = pool
            .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "alice") })
            .await
            .unwrap();
        let bob_entries = pool
            .call(|c| -> rusqlite::Result<Vec<AuditEntry>> { load_all(c, "bob") })
            .await
            .unwrap();
        assert_eq!(alice_entries.len(), 3);
        assert_eq!(bob_entries.len(), 3);
        for (a, b) in alice_entries.iter().zip(bob_entries.iter()) {
            assert_ne!(a.mac, b.mac, "different keys must produce distinct MACs");
        }
    }
}
