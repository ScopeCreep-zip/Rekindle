import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeIntentEvents } from "../ipc/channels";
import { commands } from "../ipc/commands";
import { presentReview } from "../stores/intent.store";

/**
 * Reviews of actions the backend began (`SubscriptionEvent::IntentPending`,
 * plan D3): shown in the same `IntentDialog` as a review of the window's
 * own call. The commit's answer is the Host's own; nothing here awaits it.
 * The buddy list, which may be hidden to the tray, is brought forward to
 * ask (a friend's session-reset request, a friend invite link, plan E2.1).
 */
export function subscribeBackendIntents(): Promise<UnlistenFn> {
  return subscribeIntentEvents((review) => {
    void commands.showBuddyList();
    presentReview(review).catch(() => {
      // Cancelled or expired: the Host dropped the action.
    });
  });
}
