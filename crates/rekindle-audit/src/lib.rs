#![forbid(unsafe_code)]
//! BLAKE3 keyed hash chain for tamper-evident audit logging.
//!
//! Each [`AuditEntry`] carries `prev_mac` + `mac` where
//! `mac = BLAKE3-keyed(key, prev_mac || cursor_le || payload_json)`.
//! Tampering with any byte of any entry's `payload_json` invalidates
//! every entry from that cursor forward (the chain).
//!
//! The key lives in the vault under (`"audit"`, `"mac_key"`) and is
//! generated once on first unlock. Loss of the vault loses the chain;
//! theft of the vault gains MAC-forgery capability, which is acceptable
//! because the same vault holds the identity secret it is "auditing."
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 4.

pub mod chain;

pub use chain::{AuditChain, AuditEntry, AuditKind, AuditRecord, VerifyError, MAC_LEN};
