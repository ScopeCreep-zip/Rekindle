//! The requests the GUI's window owns while the host runs in the shell
//! (plan D3.1): the identity lifecycle and the process. The GUI's login
//! pipeline unlocks, locks and changes identities until E1.1 moves it into
//! the host, and it owns the process, so a bus client cannot pull either out
//! from under the window. The window's own calls are never refused.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::DaemonContext;
use crate::host::api::{Caller, ConnId};

/// The refusal for a bus client's `request` in the shell, or `None` when it
/// may run.
pub(crate) fn refusal(
    ctx: &DaemonContext,
    request: &IpcRequest,
    caller: &Caller,
) -> Option<IpcResponse> {
    if ctx.mode != crate::host::HostMode::Shell || caller.conn == ConnId::Shell {
        return None;
    }
    let owner = ctx.require_owner().ok();
    let owned = match request {
        // Unlocking the identity the window holds attaches this client.
        IpcRequest::Unlock { identity, .. } => owner.as_deref() != Some(identity.as_str()),
        IpcRequest::IdentityDelete { identity, .. } => owner.as_deref() == Some(identity.as_str()),
        IpcRequest::Lock
        | IpcRequest::Logout
        | IpcRequest::IdentityCreate { .. }
        | IpcRequest::IdentityRotate { .. }
        | IpcRequest::IdentityWipe
        | IpcRequest::Shutdown => true,
        _ => false,
    };
    owned.then(|| {
        IpcResponse::error_with_remediation(
            409,
            format!("{} is done from the Rekindle window", request.name()),
            "use the Rekindle window, or quit it and run rekindled",
        )
    })
}
