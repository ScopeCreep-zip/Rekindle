#![forbid(unsafe_code)]
//! Idempotency cache for mutating Tauri commands.
//!
//! Mutating commands like `send_dm` are easy to fire twice by accident:
//! a user double-clicks Send, a frontend retries on network blip, an
//! optimistic UI fires before learning the previous fire succeeded. Per
//! the plan's threat model (vulnerable users, fragile networks), every
//! such duplicate must collapse into ONE actual side-effect.
//!
//! [`IdempotencyCache`] stores the response for each command keyed by a
//! caller-supplied UUID v7 (generated once per user gesture on the
//! frontend). Subsequent calls with the same key short-circuit, returning
//! the cached response instead of running the command body again.
//!
//! Plan reference: `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 8.

pub mod cache;

pub use cache::{IdempotencyCache, SharedIdempotencyCache};
