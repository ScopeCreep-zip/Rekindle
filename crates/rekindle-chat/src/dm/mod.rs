//! DM domain — 1:1 and group direct messages.
//!
//! Parameterized over `DmDeps` (orchestration) and `DmStore` (persistence).
//! No file in this module imports PlatformIO, VaultStore, SessionMeta,
//! EventPipeline, SessionCache, or MekCache directly.
//!
//! Encryption:
//! - 1:1 DMs: Triple Ratchet (per-message forward secrecy, post-quantum via PQXDH)
//! - Group DMs: AES-256-GCM via shared MEK with forward-secure ratchet chain

pub mod deps;
pub mod envelope;
pub mod error;
pub mod ingest;
pub mod invite;
pub mod mek_chain;
pub mod receiver;
pub mod sender;
pub mod session;

pub use deps::{DmDeps, DmEvent, DmMekCache};
pub use error::DmError;
pub use receiver::handle_dm_subkey_change;
pub use sender::send_dm_message;
pub use session::{accept_dm_invite, start_dm};

// DmMek and DmMekChain are internal to group MEK management.
// The public interface is the DmMekCache trait.
pub(crate) use mek_chain::DmMekChain;
