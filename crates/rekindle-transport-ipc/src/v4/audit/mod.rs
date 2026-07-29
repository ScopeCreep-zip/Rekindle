pub mod chain;
pub mod checkpoint;
pub mod gap;
pub mod replay;
pub mod replay_filter;
pub mod retention;

/// A verified audit proof received from the peer.
/// Stored by the Audit lane, consumed by application-level queries.
pub struct VerifiedProof {
    pub query_id: uuid::Uuid,
    pub session_seq: u64,
    pub link: [u8; 32],
    pub anchor_link: [u8; 32],
    pub verified: bool,
}
