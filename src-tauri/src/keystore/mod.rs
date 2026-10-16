//! The typed vault helpers the desktop uses, re-exported from
//! `rekindle_vault::typed` (plan C5.4). The vault itself is created,
//! opened and deleted by `rekindle_node::host::identity` (plan D1).

pub use rekindle_vault::typed::community_keys::{
    delete_registry_keypair, delete_slot_keypair, delete_slot_seed, load_registry_keypair,
    load_slot_keypair, load_slot_seed, persist_registry_keypair, persist_slot_keypair,
    persist_slot_seed,
};
pub use rekindle_vault::typed::mek::{
    delete_scope_meks, load_latest_mek, load_mek_generation, persist_mek,
};
pub use rekindle_vault::typed::signal::{
    delete_signal_pq_one_time, delete_signal_prekey, delete_signal_session,
    list_signal_pq_one_time_ids, list_signal_prekey_ids, list_signal_sessions,
    load_signal_pq_secret, load_signal_prekey, load_signal_session, load_signal_signed_prekey,
    load_trusted_identity, persist_signal_pq_secret, persist_signal_prekey, persist_signal_session,
    persist_signal_signed_prekey, persist_trusted_identity,
};
