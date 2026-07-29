//! Handle inbound ArenaAck — server side.
//! OK = streaming ready. REJECTED = fall back to Tier 1.
//!
//! Linux-only. The dispatch gates non-Linux with FrameDisallowedInState.

#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
use crate::v4::codec::streaming::arena_ack as codec;
#[cfg(target_os = "linux")]
use crate::v4::handlers::HandlerError;
#[cfg(target_os = "linux")]
use crate::v4::streaming::shared_arena::SharedArena;

#[cfg(target_os = "linux")]
pub fn handle(
    arenas: &mut Vec<Arc<SharedArena>>,
    conn_id: u64,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let status = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e}")))?;

    match status {
        codec::STATUS_OK => {
            tracing::info!(conn_id, "ArenaAck: client accepted — streaming ready");
        }
        codec::STATUS_REJECTED => {
            tracing::warn!(conn_id, "ArenaAck: client rejected — Tier 1 fallback");
            arenas.clear();
        }
        unknown => {
            tracing::warn!(conn_id, status = unknown, "ArenaAck: unknown — treating as rejection");
            arenas.clear();
        }
    }

    Ok(())
}
