//! Voice session commands — join, leave, status, mute, deafen.

mod join;
mod status;

use crate::cli::VoiceCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle voice <subcommand>`.
pub async fn dispatch(
    cmd: &VoiceCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        VoiceCmd::Join {
            community,
            channel,
            muted,
            deafened,
        } => join::cmd_join(handle, session, community, channel, *muted, *deafened, mode).await,
        VoiceCmd::Leave => {
            // Voice leave is a session-level operation. The active voice session
            // would be tracked in the App state (TUI) or as a one-shot operation (CLI).
            // In CLI mode, "leave" doesn't apply since voice join runs in the foreground
            // and exits on Ctrl+C.
            format::print_text("Voice leave: use Ctrl+C to exit the voice session.")
        }
        VoiceCmd::Status => status::cmd_status(handle, session, mode),
        VoiceCmd::Mute => {
            format::print_text("Mute toggle is only available during an active voice session (TUI mode).")
        }
        VoiceCmd::Deafen => {
            format::print_text("Deafen toggle is only available during an active voice session (TUI mode).")
        }
    }
}
