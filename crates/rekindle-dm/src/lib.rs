//! `rekindle-dm` — Direct messages and group DMs (architecture §27).
//!
//! DMs are SMPL records with `o_cnt: 0` (Schwarzschild — no creator
//! reserved subkeys), exactly 2 member subkeys, and a MEK derived
//! deterministically via X25519 ECDH between identity keys. This crate
//! is pure logic: types, MEK derivation, ratchet — no DHT or Tauri
//! dependencies. The `src-tauri/services/dm/` layer wires it to the
//! networking and storage stacks.

pub mod error;
pub mod invite;
pub mod mek;

pub use error::DmError;
pub use invite::{DmInvite, GroupDmInvite, GroupDmParticipant};
pub use mek::{derive_dm_mek, ratchet_dm_mek, DmMek, DmMekChain, MEK_LEN};
