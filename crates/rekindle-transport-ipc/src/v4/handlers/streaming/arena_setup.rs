//! Handle inbound ArenaSetup — client side.
//! Imports arena from pending fds, sends ArenaAck.
//!
//! Linux-only. The dispatch gates non-Linux with FrameDisallowedInState.

#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
use crate::v4::codec::streaming::{arena_ack, arena_setup as codec};
#[cfg(target_os = "linux")]
use crate::v4::handlers::HandlerError;
#[cfg(target_os = "linux")]
use crate::v4::streaming::shared_arena::SharedArena;
#[cfg(target_os = "linux")]
use crate::v4::wire::outbound::{OutboundFrame, OutboundHandoffKind};

#[cfg(target_os = "linux")]
pub fn handle(
    arenas: &mut Vec<Arc<SharedArena>>,
    pending_arena_fds: &mut Option<(std::os::unix::io::OwnedFd, std::os::unix::io::OwnedFd)>,
    sidechannel: &Option<Arc<crate::v4::streaming::sidechannel::SideChannel>>,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let setup = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e}")))?;

    if pending_arena_fds.is_none() {
        if let Some(ref sc) = *sidechannel {
            match crate::v4::streaming::sidechannel::recv_arena_fds(sc) {
                Ok((arena_fd, states_fd)) => {
                    *pending_arena_fds = Some((arena_fd, states_fd));
                }
                Err(_) => {}
            }
        }
    }

    match pending_arena_fds.take() {
        Some((arena_fd, states_fd)) => {
            match SharedArena::receive_and_import(
                arena_fd, states_fd,
                setup.slot_size as usize,
                setup.slot_count as usize,
                setup.integrity != 0,
            ) {
                Ok(arena) => {
                    let arena_id = arenas.len() as u8;
                    arenas.push(Arc::new(arena));
                    tracing::info!(arena_id, slot_size = setup.slot_size,
                        slot_count = setup.slot_count, "arena imported");
                    outbound.push(OutboundFrame::Handoff {
                        kind: OutboundHandoffKind::ArenaAck,
                        payload: arena_ack::encode(arena_ack::STATUS_OK).to_vec(),
                    });
                }
                Err(e) => {
                    tracing::warn!(?e, "arena import failed");
                    outbound.push(OutboundFrame::Handoff {
                        kind: OutboundHandoffKind::ArenaAck,
                        payload: arena_ack::encode(arena_ack::STATUS_REJECTED).to_vec(),
                    });
                }
            }
        }
        None => {
            tracing::warn!("ArenaSetup without pending fds");
            outbound.push(OutboundFrame::Handoff {
                kind: OutboundHandoffKind::ArenaAck,
                payload: arena_ack::encode(arena_ack::STATUS_REJECTED).to_vec(),
            });
        }
    }

    Ok(())
}
