import { subscribeIntentEvents, type UnlistenFn } from "../ipc/channels";
import { applyHeldReviews, presentHeldReview, presentReview } from "../stores/intent.store";

/**
 * Reviews of actions the backend began (`SubscriptionEvent::IntentPending`,
 * plan D3): shown in the same `IntentDialog` as a review of the window's
 * own call. The commit's answer is the Host's own; nothing here awaits it.
 * The Rust shell raises the buddy list (which may be hidden to the tray)
 * when the Host publishes the review (a friend's session-reset request, a
 * friend invite link, plan E2.1); this handler only renders it.
 */
export function subscribeBackendIntents(): Promise<UnlistenFn> {
  return subscribeIntentEvents((event) => {
    // The Host's complete held set (plan S7d-keys.10 part 2b): a reload or
    // re-subscribe is sent it, and the held reviews become exactly that set.
    if ("heldReviews" in event) {
      applyHeldReviews(event.heldReviews);
      return;
    }
    const { review, holdId } = event.intentPending;
    if (holdId !== null) {
      presentHeldReview({ review, holdId });
      return;
    }
    presentReview(review).catch(() => {
      // Cancelled or expired: the Host dropped the action.
    });
  });
}
