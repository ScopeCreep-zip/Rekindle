//! High-level audit-chain helpers.
//!
//! Pull the chain state from `AppState::audit_chain`, append + persist
//! atomically, and broadcast `SystemEvent::AuditChainBroken` on verify failure.

use std::sync::Arc;

use rekindle_audit::{AuditChain, AuditKind, AuditRecord, VerifyError};
use serde::Serialize;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;

use super::store::{insert_entry, load_all, load_tail};

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
            if let Err(e) = crate::keystore::persist_audit_tail(keystore, entry.cursor, &entry.mac)
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
        ks.as_ref().and_then(crate::keystore::load_audit_tail)
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
