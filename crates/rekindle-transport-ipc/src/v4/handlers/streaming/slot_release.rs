//! Handle inbound SlotRelease — writer side.
//! Reclaims slot via CAS: IN_FLIGHT(N) → FREE(N+1).
//!
//! Linux-only. The dispatch gates non-Linux with FrameDisallowedInState.

#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
use crate::v4::codec::streaming::slot_release as codec;
#[cfg(target_os = "linux")]
use crate::v4::handlers::HandlerError;
#[cfg(target_os = "linux")]
use crate::v4::streaming::shared_arena::SharedArena;

#[cfg(target_os = "linux")]
pub fn handle(
    arenas: &[Arc<SharedArena>],
    payload: &[u8],
) -> Result<(), HandlerError> {
    let release = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e}")))?;

    let arena = arenas.get(release.arena_id as usize)
        .ok_or_else(|| HandlerError::CodecFailed(format!("no arena with id {}", release.arena_id)))?;

    if !arena.return_slot(&release) {
        tracing::warn!(
            arena_id = release.arena_id,
            slot = release.slot,
            generation = release.generation,
            "SlotRelease: CAS failed"
        );
    }

    Ok(())
}
