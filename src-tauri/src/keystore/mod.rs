//! The typed vault helpers the desktop uses, re-exported from
//! `rekindle_vault::typed` (plan C5.4). The vault itself is created,
//! opened and deleted by `rekindle_node::host::identity` (plan D1).

pub use rekindle_vault::typed::community_keys::{
    delete_registry_keypair, delete_slot_keypair, delete_slot_seed, load_registry_keypair,
    load_slot_keypair, load_slot_seed, persist_registry_keypair, persist_slot_keypair,
    persist_slot_seed,
};
pub use rekindle_vault::typed::mek::{delete_scope_meks, load_mek_generation, persist_mek};
