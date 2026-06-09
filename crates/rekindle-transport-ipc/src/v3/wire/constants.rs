//! Protocol-level constants for the v3 wire format.
//!
//! Every constant here defines a byte-level property of the wire format.
//! Changes to any value are wire-incompatible and require a wire version bump.

/// Wire format version carried in every Envelope's first byte.
pub const WIRE_VERSION: u8 = 0x01;

/// Envelope size in bytes (fields + EMAC).
pub const ENVELOPE_LEN: usize = 32;

/// Stream inner header size in bytes (fields + HeaderMAC).
pub const STREAM_HEADER_LEN: usize = 32;

/// AEAD authentication tag length shared by AES-256-GCM, ChaCha20-Poly1305, AEGIS-128L.
pub const AEAD_TAG_LEN: usize = 16;

/// Envelope MAC length in bytes (truncated BLAKE3 keyed output).
pub const EMAC_LEN: usize = 16;

/// Stream header MAC length in bytes (truncated BLAKE3 keyed output).
pub const HEADER_MAC_LEN: usize = 16;

/// AEAD nonce length in bytes.
pub const AEAD_NONCE_LEN: usize = 12;

/// Direction identifier occupying nonce bytes 0..4 for Dialler-to-Listener.
/// Nonce layout: [dir_id; 4][counter; 8] = 12 bytes for AES-256-GCM.
/// The counter bytes are written as little-endian u64 by build_nonce().
/// Note: the AEAD library and kernel GCM implementation interpret the
/// 12-byte IV as three big-endian 32-bit words internally, but this is
/// transparent to the caller — both sides produce identical byte sequences.
pub const DIRECTION_ID_D2L: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Direction identifier occupying nonce bytes 0..4 for Listener-to-Dialler.
pub const DIRECTION_ID_L2D: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

// ── HKDF domain-separation labels ─────────────────────────────────
//
// Each label produces a distinct derived key from the same handshake
// hash. A typo or collision here causes silent cross-domain key reuse
// which is cryptographically catastrophic.

pub const LABEL_ENVELOPE_D2L: &str = "rti-envelope-d2l-v1";
pub const LABEL_ENVELOPE_L2D: &str = "rti-envelope-l2d-v1";
pub const LABEL_HEADER_D2L: &str = "rti-header-d2l-v1";
pub const LABEL_HEADER_L2D: &str = "rti-header-l2d-v1";
pub const LABEL_STREAM_D2L: &str = "rti-stream-d2l-v1";
pub const LABEL_STREAM_L2D: &str = "rti-stream-l2d-v1";
pub const LABEL_AUDIT_D2L: &str = "rti-audit-d2l-v1";
pub const LABEL_AUDIT_L2D: &str = "rti-audit-l2d-v1";
pub const LABEL_HANDOFF: &str = "rti-handoff-v1";

// ── Per-lane body length bounds ───────────────────────────────────

/// Control Lane minimum: 2-byte plaintext + 16-byte AEAD tag.
pub const MIN_BODY_LEN_CONTROL: u32 = 18;
/// Control Lane default maximum.
pub const MAX_BODY_LEN_CONTROL: u32 = 64 * 1024;

/// Data Lane minimum: 32-byte header + 2-byte plaintext + 16-byte tag.
pub const MIN_BODY_LEN_DATA: u32 = 50;
/// Data Lane default maximum.
pub const MAX_BODY_LEN_DATA: u32 = 16 * 1024 * 1024;

/// Audit Lane minimum.
pub const MIN_BODY_LEN_AUDIT: u32 = 18;
/// Audit Lane default maximum.
pub const MAX_BODY_LEN_AUDIT: u32 = 1024 * 1024;

/// Handoff Lane minimum.
pub const MIN_BODY_LEN_HANDOFF: u32 = 18;
/// Handoff Lane default maximum.
pub const MAX_BODY_LEN_HANDOFF: u32 = 4 * 1024;

/// UCred prologue prefix bound into the Noise handshake hash.
pub const PROLOGUE_PREFIX: &str = "RTI-IPC-v1:";

/// Maximum bytes per writev segment. Bounds priority-preemption latency.
pub const WRITE_CHUNK_MAX: usize = 64 * 1024;
