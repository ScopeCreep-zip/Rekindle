//! Cryptographic key management commands — MEK, prekeys, inspect.

mod inspect;
mod mek;
mod prekeys;

use crate::cli::KeyCmd;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle key <subcommand>`.
pub async fn dispatch(
    cmd: &KeyCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        KeyCmd::Mek(mek_cmd) => mek::dispatch(mek_cmd, handle, session, mode).await,
        KeyCmd::Prekeys(prekey_cmd) => prekeys::dispatch(prekey_cmd, handle, session, mode).await,
        KeyCmd::Inspect { community } => {
            inspect::cmd_inspect(handle, session, community, mode)
        }
    }
}
