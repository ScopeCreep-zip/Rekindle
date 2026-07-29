//! Handle inbound SharedMemRef (ArenaWrite) — reader side.
//!
//! Validates the ref, reads payload from arena (zero-copy), delivers
//! to FrameRouter, sends SlotRelease back to writer.
//!
//! Integrity check flag is read from the arena itself (per-arena
//! configuration). The arena_id is at byte 0 of the payload.
//!
//! Linux-only. The dispatch gates non-Linux with FrameDisallowedInState.

#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
use crate::v4::codec::streaming::{self, arena_write as codec};
#[cfg(target_os = "linux")]
use crate::v4::handlers::HandlerError;
#[cfg(target_os = "linux")]
use crate::v4::router::{ConnectionInfo, FrameRouter};
#[cfg(target_os = "linux")]
use crate::v4::streaming::{SlotRelease, shared_arena::SharedArena};
#[cfg(target_os = "linux")]
use crate::v4::wire::outbound::{OutboundFrame, OutboundHandoffKind};

#[cfg(target_os = "linux")]
pub fn handle(
    arenas: &[Arc<SharedArena>],
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    tracing::debug!(
        payload_len = payload.len(),
        arena_count = arenas.len(),
        "arena_write handler: ENTER"
    );

    if payload.is_empty() {
        return Err(HandlerError::CodecFailed("ArenaWrite payload empty".into()));
    }
    let arena_id = payload[0];
    let arena = arenas.get(arena_id as usize)
        .ok_or_else(|| {
            tracing::error!(arena_id, arena_count = arenas.len(), "arena_write: no arena with this id");
            HandlerError::CodecFailed(format!("no arena with id {arena_id}"))
        })?;

    let integrity = arena.integrity_check();
    let shmref = codec::decode(payload, integrity)
        .map_err(|e| {
            tracing::error!(arena_id, error = %e, "arena_write: decode failed");
            HandlerError::CodecFailed(format!("{e}"))
        })?;

    tracing::debug!(
        arena_id = shmref.arena_id,
        slot = shmref.slot,
        generation = shmref.generation,
        length = shmref.length,
        has_digest = shmref.digest.is_some(),
        "arena_write: validating read_ref"
    );

    let data = arena.read_ref(&shmref)
        .ok_or_else(|| {
            tracing::error!(
                arena_id = shmref.arena_id,
                slot = shmref.slot,
                generation = shmref.generation,
                length = shmref.length,
                "arena_write: read_ref validation FAILED"
            );
            HandlerError::CodecFailed("SharedMemRef validation failed".into())
        })?;

    tracing::debug!(data_len = data.len(), slot = shmref.slot, "arena_write: delivering to router");
    router.on_arena_write(info, &shmref, data);

    let release = SlotRelease {
        arena_id: shmref.arena_id,
        slot: shmref.slot,
        generation: shmref.generation,
    };
    let mut buf = [0u8; streaming::slot_release::WIRE_SIZE];
    streaming::slot_release::encode(&release, &mut buf);
    outbound.push(OutboundFrame::Handoff {
        kind: OutboundHandoffKind::SlotRelease,
        payload: buf.to_vec(),
    });

    Ok(())
}
