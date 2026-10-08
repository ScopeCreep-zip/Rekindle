//! Phase 22.e-REDO — cross-device sync primitives.
//!
//! Pure CRDT merge rules for the personal sync record (architecture
//! §28.4). AppState + DHT IO (pairing handshake, watch loop, subkey
//! read/write) stays in src-tauri behind the existing
//! `cross_device_sync` facade per the chiral split pattern.

pub mod merge;
pub mod util;

pub use merge::{merge_device_list, merge_manifest, merge_preferences, merge_read_state};
pub use util::{classify_remote_subkey, generate_device_id, RemoteSubkeyDecoded};
