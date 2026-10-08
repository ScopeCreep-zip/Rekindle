pub mod cross_device;
pub mod deps;
pub mod fetch;
pub mod gap;
pub mod history;
pub mod inspect;
#[cfg(test)]
mod path_independence;
pub mod pending_retry;
pub mod verify;
pub mod warming;
pub mod watch;

pub use cross_device::{
    classify_remote_subkey, generate_device_id, merge_device_list, merge_manifest,
    merge_preferences, merge_read_state, RemoteSubkeyDecoded,
};
pub use deps::{PendingMessageRow, PendingRetryOutcome, SyncDeps};
pub use pending_retry::{
    is_retry_eligible, process_pending_retry_queue, should_drop_pending, MAX_PENDING_RETRIES,
    SYNC_LOOP_INTERVAL_MS,
};
