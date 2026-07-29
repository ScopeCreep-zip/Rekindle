//! Server-side arena bootstrap — create SharedArena + sidechannel,
//! send fds to client, build ArenaBootstrap for connection setup.

use std::sync::Arc;

use crate::v4::config::SessionConfig;
use crate::v4::io::connection::ArenaBootstrap;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::outbound::OutboundFrame;

/// Create the shared-memory arena and SEQPACKET sidechannel for a server connection.
/// Returns (arena, sidechannel) or (None, None) on failure.
#[cfg(target_os = "linux")]
pub fn create_arena(
    stream: &tokio::net::UnixStream,
    config: &SessionConfig,
    active_capabilities: CapabilityBits,
    conn_id: u64,
) -> (
    Option<Arc<crate::v4::streaming::shared_arena::SharedArena>>,
    Option<Arc<crate::v4::streaming::sidechannel::SideChannel>>,
) {
    if !active_capabilities.contains(CapabilityBits::SHARED_ARENA) {
        return (None, None);
    }
    use std::os::unix::io::AsRawFd;
    use crate::v4::streaming::sidechannel;
    use crate::v4::streaming::shared_arena::SharedArena;

    let (server_end, client_end) = match sidechannel::SideChannel::create_pair() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(conn_id, error = ?e, "sidechannel: socketpair failed");
            return (None, None);
        }
    };
    if let Err(e) = sidechannel::send_fd_over_stream(stream.as_raw_fd(), client_end.fd()) {
        tracing::warn!(conn_id, error = ?e, "sidechannel: send failed");
        return (None, None);
    }
    drop(client_end);

    let arena = match SharedArena::create(
        config.arena_slot_size, config.arena_slot_count, config.arena_integrity_check,
    ) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(conn_id, error = ?e, "arena: create failed");
            return (None, None);
        }
    };
    if let Err(e) = sidechannel::send_arena_fds(&server_end, arena.arena_fd(), arena.states_fd()) {
        tracing::warn!(conn_id, error = ?e, "arena: fd send failed");
        return (None, None);
    }
    tracing::debug!(conn_id, "arena: fds sent via sidechannel");
    (Some(Arc::new(arena)), Some(Arc::new(server_end)))
}

/// Build ArenaBootstrap::Server from the created arena + sidechannel.
#[cfg(target_os = "linux")]
pub fn build_bootstrap(
    arena: &Option<Arc<crate::v4::streaming::shared_arena::SharedArena>>,
    sidechannel: &Option<Arc<crate::v4::streaming::sidechannel::SideChannel>>,
    config: &SessionConfig,
) -> ArenaBootstrap {
    match arena {
        Some(a) => {
            let setup = crate::v4::codec::streaming::arena_setup::ArenaSetupPayload {
                slot_size: a.slot_size() as u32,
                slot_count: a.slot_count() as u16,
                integrity: if config.arena_integrity_check { 1 } else { 0 },
            };
            let setup_bytes = crate::v4::codec::streaming::arena_setup::encode(&setup);
            ArenaBootstrap::Server {
                arenas: vec![Arc::clone(a)],
                sidechannel: sidechannel.clone(),
                initial_outbound: vec![OutboundFrame::Handoff {
                    kind: crate::v4::wire::frame_kind::HandoffKind::ArenaSetup,
                    payload: setup_bytes.to_vec(),
                }],
            }
        }
        None => ArenaBootstrap::None,
    }
}
