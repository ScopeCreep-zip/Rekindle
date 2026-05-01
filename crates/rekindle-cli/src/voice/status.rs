//! `rekindle voice status` — show current voice session info.

use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Show current voice session status.
///
/// In CLI one-shot mode, there's no persistent voice session — `voice join`
/// runs in the foreground and exits on Ctrl+C. This command reports
/// the node's voice-related state: whether any voice routes are allocated,
/// and the community/channel if known.
pub fn cmd_status(
    handle: &TransportHandle,
    _session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // The CLI doesn't maintain a persistent voice session between commands.
    // Voice status is meaningful in the TUI where the session persists.
    // Here we report what we can from the transport node.

    let node_status = handle.node().status_snapshot();

    if mode.is_structured() {
        return format::print_structured(
            &serde_json::json!({
                "voice_session": null,
                "node_attached": node_status.is_attached,
                "peers": node_status.peer_count,
            }),
            mode,
        );
    }

    format::print_text("No active voice session.")?;
    format::print_text(&format!(
        "  Node: {} ({} peers)",
        if node_status.is_attached {
            "[ONLINE]"
        } else {
            "[OFFLINE]"
        },
        node_status.peer_count
    ))?;
    format::print_text("")?;
    format::print_text(
        "Join a voice channel: rekindle voice join -c \"community\" -C \"channel\"",
    )
}
