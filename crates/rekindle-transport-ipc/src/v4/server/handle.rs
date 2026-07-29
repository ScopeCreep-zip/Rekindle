//! ConnectionHandle — per-connection public API given to the router factory.

use tokio::sync::mpsc;

use crate::v4::bulk::send::BulkSender;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::outbound::OutboundFrame;

/// Per-connection handle given to the router factory after handshake.
///
/// The factory uses this to construct a per-connection FrameRouter that
/// owns the outbound channels. When on_request fires, the router sends
/// DATAGRAM_REPLY through outbound_tx. When a large response is needed,
/// the router uses bulk_sender for rayon-parallel encryption.
#[derive(Clone)]
pub struct ConnectionHandle {
    /// Send control-plane frames (DATAGRAM_REPLY, events, lifecycle) to this client.
    /// Unbounded: replies to admitted requests must never be dropped.
    /// Memory bounded by max_pending_requests (admission control).
    pub outbound_tx: mpsc::UnboundedSender<OutboundFrame>,
    /// Send bulk data to this client via rayon parallel encryption.
    pub bulk_sender: BulkSender,
    /// Tier-aware streaming sender — routes to arena or inline automatically.
    #[cfg(target_os = "linux")]
    pub streaming_sender: Option<crate::v4::streaming::send::StreamingSender>,
    /// Connection metadata.
    pub conn_id: u64,
    pub session_id: uuid::Uuid,
    pub peer_id: [u8; 32],
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
}
