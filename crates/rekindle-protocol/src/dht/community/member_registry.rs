//! Member registry operations for SMPL multi-writer DHT records.
//!
//! Communities v2.0 registry layout (`DHTSchema::smpl(0, members)` —
//! architecture "no node above another"): the creation keypair is
//! discarded after genesis and there are NO owner subkeys. Every
//! subkey 0..254 is a member slot; members claim a slot
//! self-sovereignly (join flow) and write their own `MemberPresence`
//! to it with the slot keypair derived from the shared slot seed.
//!
//! Presence reads/writes address slots RAW — the v1.0 coordinator-era
//! `+REGISTRY_OWNER_SUBKEY_COUNT` offset is gone. Record creation
//! lives in the governance runtime (`create_smpl_record` with
//! `rekindle_records::schema::community_smpl`); this module keeps only
//! the layout's tests.

// The slot keypairs derive from the slot seed through
// `rekindle_secrets::derive::derive_slot_keypair`; a slot writer's keypair
// text is `rekindle_crypto::keypair_text::slot_keypair_text` (plan C9.1b).

#[cfg(test)]
#[path = "member_registry_layout_invariants.rs"]
mod layout_invariants;
