//! Backend-originated intents (ADR 0019): a review the Host raises itself,
//! with no window asking, held in the intent gate and announced as
//! `IntentPending` to every frontend, and handed in the held set
//! (`SubscriptionEvent::HeldReviews`) to every window that subscribes
//! while it is held. The friend adapter's session-reset,
//! re-initiation and identity-rotation holds and the community
//! `ReEnrolDevice` review (plan E3.2 S7d-keys.10) share it.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::intent::IntentKind;
use rekindle_types::subscription_events::SubscriptionEvent;

use crate::daemon::dispatch::DaemonContext;
use crate::host::gate::IntentOwner;

/// Why a backend hold was not raised.
#[derive(Debug)]
pub(crate) enum HoldRefused {
    /// No identity is unlocked.
    Locked,
    /// The review could not be built (its selection's answer).
    Select(String),
    /// The gate refused it (full, or the hold failed).
    Gate(String),
}

/// Hold `action` (with `cancel`, run on Cancel) under `kind` and announce
/// it. An unchanged selection for the same action keeps its hold. A changed
/// selection replaces the older hold without running its cancel, provided
/// the new hold fits (plan audit D-4).
///
/// # Errors
/// [`HoldRefused`].
pub(crate) async fn raise(
    ctx: &DaemonContext,
    action: IpcRequest,
    cancel: Option<IpcRequest>,
    kind: IntentKind,
    same: fn(&IpcRequest, &IpcRequest) -> bool,
) -> Result<(), HoldRefused> {
    let owner = IntentOwner {
        conn: None,
        identity: ctx.require_owner().map_err(|_| HoldRefused::Locked)?,
    };
    let selection = crate::host::intent_select::select(ctx, &action, kind)
        .await
        .map_err(|e| HoldRefused::Select(format!("{e:?}")))?;
    // Keep concurrent raises' events in the same order as their gate changes.
    let _publication = ctx.backend_hold_publication.lock();
    if ctx.require_owner().map_err(|_| HoldRefused::Locked)? != owner.identity {
        return Err(HoldRefused::Locked);
    }
    let Some((review, replaced)) = ctx
        .intents
        .hold_backend(owner, selection, action, cancel, same, ctx.clock.now_ms())
        .map_err(|e| HoldRefused::Gate(e.to_string()))?
    else {
        return Ok(());
    };
    if replaced {
        ctx.publish_event(SubscriptionEvent::HeldReviews(super::held_reviews_of(ctx)));
    }
    // Live to the windows open now; a window that subscribes later (or
    // resumes) is handed the held set instead (`Host::held_reviews`).
    let hold_id = Some(review.intent_id);
    ctx.publish_event(SubscriptionEvent::IntentPending { review, hold_id });
    Ok(())
}

#[cfg(test)]
#[path = "backend_hold_tests.rs"]
mod tests;
