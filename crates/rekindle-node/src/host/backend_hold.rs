//! Backend-originated intents (ADR 0019): a review the Host raises itself,
//! with no window asking, held in the intent gate and announced as
//! `IntentPending` to every frontend. The friend adapter's session-reset,
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
/// it. One hold per `same` action: a newer one replaces the older, which
/// goes without its cancel, but only once the newer fits: a full gate
/// refuses it and keeps the older (plan audit D-4).
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
    let now = ctx.clock.now_ms();
    if !ctx
        .intents
        .has_room(&owner, |held_action| same(held_action, &action), now)
    {
        return Err(HoldRefused::Gate(
            rekindle_idempotency::intent_gate::GateError::TooManyPending.to_string(),
        ));
    }
    ctx.intents.retain_held(|held_by, held_action| {
        !(held_by.conn.is_none()
            && held_by.identity == owner.identity
            && same(held_action, &action))
    });
    let review = ctx
        .intents
        .hold(
            owner,
            selection,
            action,
            cancel,
            rekindle_idempotency::intent_gate::BACKEND_HOLD_TTL_MS,
            now,
        )
        .map_err(|e| HoldRefused::Gate(e.to_string()))?;
    ctx.publish_event(SubscriptionEvent::IntentPending(review));
    Ok(())
}
