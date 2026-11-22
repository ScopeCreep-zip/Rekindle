//! `SkinApplyFailed` (plan SK2-T1): a window could not apply a skin (an
//! image that did not decode, a stylesheet that did not load) and kept
//! the skin it shows. The window only reports; the Host decides (lead
//! decisions 51, 52): it logs the failure, tells that window and Settings
//! (`SystemEvent::SkinApplyFailed`), and ends a trial of that skin at
//! once, so a library skin that does not render is not left on screen
//! for the rest of "Keep this skin?".

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::trial;
use crate::daemon::dispatch::DaemonContext;

/// The router's arm for `SkinApplyFailed`. Answered before unlock too
/// (the login window's device skin); a trial needs an unlocked identity.
pub(crate) async fn report(ctx: &DaemonContext, request: IpcRequest) -> IpcResponse {
    let IpcRequest::SkinApplyFailed {
        window,
        skin,
        cause,
    } = request
    else {
        unreachable!("{} is not SkinApplyFailed", request.name());
    };
    tracing::warn!(%window, %skin, ?cause, "a window could not apply a skin");
    let reverted = match ctx.require_owner() {
        Ok(owner) => trial::revert_failed(ctx, &owner, &skin).await,
        Err(_) => false,
    };
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::SkinApplyFailed {
        window,
        skin,
        cause,
        reverted,
    }));
    IpcResponse::ok(&())
}
