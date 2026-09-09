import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeCommunityEvents } from "../../ipc/channels";
import { reduceMembership, reduceSubscriptionMembership } from "./dispatcher_members";
import { isLegacyCommunityEvent } from "../../ipc/channels/community_subscription_events";
import { reduceMessages } from "./dispatcher_messages";
import { reduceVoice } from "./dispatcher_voice";
import { reduceContent } from "./dispatcher_content";

/// Central community event dispatcher. Each incoming `CommunityEvent`
/// is offered to the topic reducers in turn; the first one that
/// recognises the event type consumes it (event types are mutually
/// exclusive so ordering is incidental). The slices are split to keep
/// every dispatcher file under the module size cap.
export function subscribeCommunityEventDispatcher(): Promise<UnlistenFn> {
  return subscribeCommunityEvents((event) => {
    // The daemon vocabulary and the desktop envelope share this
    // channel while the migration is in progress. Membership has moved
    // across; the rest still arrives as `{ type, data }`.
    if (!isLegacyCommunityEvent(event)) {
      reduceSubscriptionMembership(event);
      return;
    }
    if (reduceMembership(event)) return;
    if (reduceMessages(event)) return;
    if (reduceVoice(event)) return;
    reduceContent(event);
  });
}
