//! Phase 15 — Files domain adapter.
//!
//! Implements `rekindle_files::FilesDeps` against the live `AppState`
//! + `tauri::AppHandle` + `DbPool`. The crate's upload / download /
//! expression_fetch flows parameterise over this trait so the protocol
//! logic stays free of Tauri/Veilid concerns (Invariant 2).
//!
//! All ~28 trait methods are real — the 24 sync methods touch
//! AppState directly (identity, cache, community state, MEK lookup,
//! permissions, lamport/sequence, governance reads, gossip mesh, event
//! emit) and the 9 async methods bridge to Veilid (DHT writes/scans,
//! app_call), SQLite (insert_channel_message_full, persist_local_path),
//! and the governance write path (write_attachment_pinned).
//!
//! Phase 15.r split layout (Invariant 1, ≤500 LoC per file):
//! * [`deps_impl`] — full `impl FilesDeps for FilesAdapter` block;
//!   bigger method bodies delegate into `helpers`.
//! * [`helpers`] — extracted bodies that would otherwise blow the
//!   500-LoC behavior cap in the trait impl (DHT-writer prep,
//!   3-tier MEK cascade, DB inserts, slowmode persist, event mapping).

use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;

pub mod deps_impl;
pub mod helpers;

pub struct FilesAdapter {
    pub(crate) state: Arc<AppState>,
    pub(crate) app_handle: tauri::AppHandle,
    pub(crate) pool: DbPool,
}

impl FilesAdapter {
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: DbPool) -> Arc<Self> {
        Arc::new(Self {
            state,
            app_handle,
            pool,
        })
    }
}
