//! Nonce domain separation tests.
//!
//! These catch the catastrophic nonce-reuse-across-directions bug
//! that silently enables AES-GCM key-stream XOR recovery.

mod direction_ids;
