//! Named payload structs for the largest `ControlPayload` variants.
//!
//! serde uses DEFAULTS here (snake_case fields, no rename_all) — this
//! enum family's wire is PascalCase tags + snake_case fields, and the
//! newtype conversion must byte-match it (see `wire_tests.rs`).

use rekindle_types::video::Codec;
use serde::{Deserialize, Serialize};

/// Direct app_call delivery of wrapped MEK material to a single peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekTransferPayload {
    pub community_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub generation: u64,
    pub sender_pseudonym: String,
    pub wrapped_mek: Vec<u8>,
}

/// P1.3 — requester → responder ack confirming successful
/// `MekTransfer` ingestion. Sent as the `app_call` reply by the
/// MekTransfer receiver after MEK unwrap succeeds. Lets the
/// responder distinguish app-layer success (decryption worked)
/// from network-layer success (packet arrived) so a misrouted or
/// generation-mismatched transfer surfaces in observability
/// rather than disappearing into a synchronous `ACK`.
///
/// Wire format: Cap'n Proto `MekTransferAckPayload` (ordinal 67 in
/// `community_envelope.capnp`). Forward-compatible: peers built
/// before this variant existed see `Which::NotInSchema` and fall
/// back to treating the reply as a bare ACK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MekTransferAckPayload {
    pub community_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    pub generation: u64,
    /// Hex pseudonym of the receiver (us, the requester) — lets
    /// the responder match the ack against in-flight transfers
    /// indexed by `(community_id, channel_id, generation,
    /// requester_pseudonym)`.
    pub requester_pseudonym: String,
}

/// Architecture §10.6 video / screen-share fragment. Frames are
/// encoded with `codec`, MEK-encrypted, then split into ≤28 KB
/// chunks so they fit inside Veilid `app_message`. The 16-byte
/// `stream_id` is `blake3(channel_id || sender_pseudonym)[..16]` so
/// concurrent streams in the same channel never collide. Reassembly
/// happens in `rekindle-video::Reassembler` on the receive side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFragmentPayload {
    pub channel_id: String,
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub frag_index: u8,
    pub frag_total: u8,
    pub keyframe: bool,
    /// RTP payload-type analog — receivers configure their decoder
    /// from this tag. Signature-covered.
    pub codec: Codec,
    pub timestamp: u32,
    /// Generation of the channel-media MEK that encrypted the
    /// frame — receivers request exactly this generation on
    /// decrypt failure instead of guessing. Signature-covered.
    pub mek_generation: u64,
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Forward-error-correction parity packet for the matching
/// `VideoFragment` data stream (architecture §10.6 line 4080).
/// Modelled on RFC 5109 / FlexFEC: a separate packet variant
/// carrying Reed-Solomon parity shards so a few dropped data
/// fragments don't force a full keyframe re-request. Receivers
/// without FEC support ignore these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoParityFragmentPayload {
    pub channel_id: String,
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub parity_index: u8,
    pub parity_total: u8,
    pub data_count: u8,
    /// Codec of the frame this parity covers. Signature-covered.
    pub codec: Codec,
    pub frame_len: u32,
    pub timestamp: u32,
    /// Mirrors `VideoFragment::mek_generation`. Signature-covered.
    pub mek_generation: u64,
    pub payload: Vec<u8>,
    pub signature: Vec<u8>,
}
