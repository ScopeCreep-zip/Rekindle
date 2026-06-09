//! Layer 1: Wire types — byte-level structures and constants.
//!
//! This module is the ONLY place in the crate where byte offsets,
//! field widths, lane values, FrameKind values, and wire constants
//! are defined. No other module may hard-code a byte offset or a
//! discriminant value.
//!
//! # Invariants
//!
//! - `EnvelopeWire` is exactly 32 bytes.
//! - `StreamHeaderWire` is exactly 32 bytes.
//! - `EnvelopeWire` + `StreamHeaderWire` = 64 bytes = one L1 cache line.
//! - All multi-byte integers are little-endian on the wire.
//! - All `u32` fields are at 4-byte-aligned offsets.
//! - All `u64` fields are at 8-byte-aligned offsets.

pub mod constants;
pub mod envelope;
pub mod header;
pub mod lane;
pub mod frame_class;
pub mod frame_kind;
pub mod capability;
pub mod clearance;
pub mod failure;
