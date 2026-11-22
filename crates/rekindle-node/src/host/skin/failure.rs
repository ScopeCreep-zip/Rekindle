//! `SkinApplyFailed` (plan SK2-T1): a window could not apply a skin (an
//! image that did not decode, a stylesheet that did not load) and kept
//! the skin it shows. The window only reports; the Host decides (lead
//! decisions 51, 52): it logs the failure, tells that window and Settings
//! (`SystemEvent::SkinApplyFailed`), and ends a trial of that skin at
//! once, so a library skin that does not render is not left on screen
//! for the rest of "Keep this skin?".
//!
//! Every window applying a skin reports its own failure, so one broken
//! image is N reports. The Host tells it once: a (skin, cause) already
//! told is only logged at DEBUG until that skin is applied afresh
//! ([`ToldFailures::applying`], from every `SkinChanged`) or the identity
//! locks. A different cause of the same skin is a new failure.

use std::collections::HashSet;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_library::SkinApplyFailure;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::trial;
use crate::daemon::dispatch::DaemonContext;

/// The (skin, cause) pairs already told (`SkinState::failures`).
#[derive(Default)]
pub(crate) struct ToldFailures {
    told: parking_lot::Mutex<HashSet<(String, SkinApplyFailure)>>,
}

impl ToldFailures {
    /// True if this is the first report of `cause` for `skin` since that
    /// skin was last applied; it is then held as told.
    fn first(&self, skin: &str, cause: &SkinApplyFailure) -> bool {
        self.told.lock().insert((skin.to_owned(), cause.clone()))
    }

    /// The windows apply `skin` afresh (a choice, a trial, a revert, a
    /// re-apply): its failures are new again.
    pub(crate) fn applying(&self, skin: &str) {
        self.told.lock().retain(|(told, _)| told != skin);
    }

    /// The identity locks: nothing it was told is held.
    pub(crate) fn clear(&self) {
        self.told.lock().clear();
    }
}

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
    if !ctx.skin.failures.first(&skin, &cause) {
        tracing::debug!(%window, %skin, ?cause, "another window could not apply a skin");
        // Idempotent: the first report already ended a trial of it.
        if let Ok(owner) = ctx.require_owner() {
            trial::revert_failed(ctx, &owner, &skin).await;
        }
        return IpcResponse::ok(&());
    }
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
