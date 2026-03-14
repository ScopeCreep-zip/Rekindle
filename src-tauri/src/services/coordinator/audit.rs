//! Audit log writer for the coordinator.
//!
//! The coordinator writes signed audit entries for moderation actions to a
//! dedicated DHT record (DFLT 256 subkeys, ring buffer). The record key is
//! stored in manifest subkey 14.

use std::sync::Arc;

use parking_lot::Mutex;

use rekindle_protocol::dht::community::audit_log::{
    AuditAction, AuditChange, AuditLogEntry, AuditTarget,
};
use rekindle_protocol::dht::DHTManager;

use crate::state::AppState;
use crate::state_helpers;

/// Maximum serialized size per subkey page (~30KB, leaving room for DHT overhead).
const MAX_PAGE_SIZE: usize = 30_000;

/// Number of subkeys in the audit record (ring buffer).
const AUDIT_SUBKEY_COUNT: u16 = 256;

/// Audit logger that writes entries to a DHT ring buffer.
pub struct AuditLogger {
    /// DHT record key for the audit log.
    audit_record_key: Option<String>,
    /// Current page (subkey) being written to.
    current_page: u32,
    /// Next sequential entry ID.
    next_entry_id: u64,
    /// Accumulated entries for the current page.
    page_buffer: Vec<AuditLogEntry>,
    /// Accumulated serialized size of the current page.
    page_size: usize,
}

impl AuditLogger {
    /// Create a new audit logger (audit record not yet created).
    pub fn new() -> Self {
        Self {
            audit_record_key: None,
            current_page: 0,
            next_entry_id: 1,
            page_buffer: Vec::new(),
            page_size: 0,
        }
    }

    /// Set the audit record key (loaded from manifest subkey 14).
    pub fn set_record_key(&mut self, key: String) {
        self.audit_record_key = Some(key);
    }

    /// Get the audit record key, if created.
    pub fn record_key(&self) -> Option<&str> {
        self.audit_record_key.as_deref()
    }
}

/// Log an audit action.
///
/// Appends an entry to the current page. When the page exceeds `MAX_PAGE_SIZE`,
/// advances to the next subkey (wrapping at 256).
pub async fn log_action(
    state: &Arc<AppState>,
    community_id: &str,
    logger: &Mutex<AuditLogger>,
    action: AuditAction,
    target: AuditTarget,
    changes: Vec<AuditChange>,
    reason: Option<String>,
) {
    let now_secs = rekindle_utils::timestamp_secs();

    // Build the entry, signed with coordinator pseudonym key
    let (entry, audit_key, current_page) = {
        let mut log = logger.lock();

        // Lazily create audit record if needed
        if log.audit_record_key.is_none() {
            // Will be created on first use — for now just skip
            tracing::debug!(
                community = %community_id,
                "audit log not initialized, skipping entry"
            );
            return;
        }

        let mut entry = AuditLogEntry {
            entry_id: log.next_entry_id,
            actor_pseudonym: get_my_pseudonym(state, community_id),
            action,
            target,
            changes,
            reason,
            timestamp: now_secs,
            signature: Vec::new(),
        };

        // Sign with coordinator pseudonym Ed25519 key
        entry.signature = sign_audit_entry(state, community_id, &entry);

        log.next_entry_id += 1;

        // Check if we need to advance to next page
        let entry_size = serde_json::to_vec(&entry).map_or(0, |b| b.len());
        if log.page_size + entry_size > MAX_PAGE_SIZE {
            // Advance to next page (ring buffer)
            log.current_page = (log.current_page + 1) % u32::from(AUDIT_SUBKEY_COUNT);
            log.page_buffer.clear();
            log.page_size = 0;
        }

        log.page_buffer.push(entry.clone());
        log.page_size += entry_size;

        let key = log.audit_record_key.clone().unwrap_or_default();
        let page = log.current_page;
        (entry, key, page)
    };

    // Write the page to DHT
    let Some(rc) = state_helpers::routing_context(state) else {
        return;
    };
    let mgr = DHTManager::new(rc);

    let page_entries = {
        let log = logger.lock();
        log.page_buffer.clone()
    };

    let page_bytes = match serde_json::to_vec(&page_entries) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize audit page");
            return;
        }
    };

    if let Err(e) = mgr.set_value(&audit_key, current_page, page_bytes).await {
        tracing::warn!(
            community = %community_id,
            page = current_page,
            error = %e,
            "failed to write audit log page"
        );
    } else {
        tracing::debug!(
            community = %community_id,
            entry_id = entry.entry_id,
            action = ?entry.action,
            page = current_page,
            "audit log entry written"
        );
    }
}

/// Read audit log entries from the DHT record.
pub async fn read_entries(
    state: &Arc<AppState>,
    audit_key: &str,
    limit: usize,
) -> Result<Vec<AuditLogEntry>, String> {
    let rc = state_helpers::routing_context(state).ok_or("not attached")?;
    let mgr = DHTManager::new(rc);

    let mut all_entries = Vec::new();

    // Scan pages in reverse order to get most recent entries first
    for page in (0..u32::from(AUDIT_SUBKEY_COUNT)).rev() {
        match mgr.get_value(audit_key, page).await {
            Ok(Some(data)) => {
                if let Ok(entries) = serde_json::from_slice::<Vec<AuditLogEntry>>(&data) {
                    for entry in entries.into_iter().rev() {
                        all_entries.push(entry);
                        if all_entries.len() >= limit {
                            return Ok(all_entries);
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(page, error = %e, "failed to read audit page");
            }
        }
    }

    Ok(all_entries)
}

fn get_my_pseudonym(state: &Arc<AppState>, community_id: &str) -> String {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|c| c.my_pseudonym_key.clone())
        .unwrap_or_default()
}

/// Sign an audit log entry with the coordinator's pseudonym Ed25519 key.
///
/// Serializes the entry (with empty signature field) and signs with the
/// pseudonym signing key derived from `identity_secret + community_id`.
fn sign_audit_entry(
    state: &Arc<AppState>,
    community_id: &str,
    entry: &AuditLogEntry,
) -> Vec<u8> {
    // Get identity secret from state
    let Some(secret) = *state.identity_secret.lock() else {
        return Vec::new();
    };

    // Derive the pseudonym signing key for this community
    let signing_key =
        rekindle_crypto::group::pseudonym::derive_community_pseudonym(&secret, community_id);

    // Serialize entry with empty signature for signing
    let mut signable = entry.clone();
    signable.signature = Vec::new();
    let Ok(bytes) = serde_json::to_vec(&signable) else {
        return Vec::new();
    };

    rekindle_crypto::group::pseudonym::sign_with_pseudonym(&signing_key, &bytes).to_vec()
}
