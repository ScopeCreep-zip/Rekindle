//! Architecture §28.4 — cross-device sync orchestration.
//!
//! The personal DFLT record holds the 4 well-known subkeys defined in
//! `rekindle-types::cross_device_sync` (manifest / read state /
//! preferences / device list). This module owns:
//!
//! * record creation + opening (`record`)
//! * encrypt-then-write / read-then-decrypt for each subkey
//!   (`subkey_io`)
//! * watch-and-merge loop (`watch`)
//! * pairing handshake (`pairing`)
//! * merge rules per subkey (`merge`)

mod merge;
mod pairing;
mod record;
mod subkey_io;
pub(crate) mod watch;

pub use pairing::{
    build_pairing_payload, generate_pairing_session, handle_pairing_app_call, PairingSession,
};
pub use record::{
    ensure_personal_sync_record, open_personal_sync_record, PersonalSyncRecordHandle,
};
pub use subkey_io::{
    read_device_list, read_preferences, read_read_state, read_sync_manifest, write_device_list,
    write_preferences, write_read_state, write_sync_manifest,
};
pub use watch::start_personal_sync_watch;

#[cfg(test)]
mod tests;
