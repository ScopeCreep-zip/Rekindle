import type { DeliveryRetry } from "../ipc/channels/chat_events";

/**
 * A send the Host keeps retrying, worded (plan SR2): a peer whose routes
 * are all lost answers again only once Veilid releases them, so a stalled
 * message says so, with the Host's attempt count and next try, rather than
 * reading "sent" while nothing arrives.
 */
export function deliveryRetryText(retry: DeliveryRetry): string {
  const at = new Date(retry.nextAttemptAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  return `Not delivered yet — still trying (attempt ${retry.attempts}, next try at ${at})`;
}
