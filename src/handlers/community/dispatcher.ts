import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeCommunityEvents, subscribeVoiceEvents } from "../../ipc/channels";
import { reduceMembership, reduceSubscriptionMembership } from "./dispatcher_members";
import { isLegacyCommunityEvent } from "../../ipc/channels/community_subscription_events";
import {
  reduceMessages,
  reduceSubscriptionMessages,
  reduceSubscriptionSystem,
  reduceSubscriptionTyping,
} from "./dispatcher_messages";
import {
  reduceVoice,
  reduceSubscriptionCrypto,
  reduceSubscriptionVoice,
} from "./dispatcher_voice";
import { reduceSubscriptionContent } from "./dispatcher_content";

const SUBSCRIPTION_SLICES = [
  reduceSubscriptionMembership,
  reduceSubscriptionMessages,
  reduceSubscriptionSystem,
  reduceSubscriptionTyping,
  reduceSubscriptionCrypto,
  reduceSubscriptionContent,
];

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
      // Each slice takes the families it owns and ignores the rest;
      // unlike the legacy reducers these do not report consumption,
      // because the daemon families are disjoint by construction.
      // A slice that throws is reported and the others still run.
      for (const reduce of SUBSCRIPTION_SLICES) {
        try {
          reduce(event);
        } catch (err) {
          console.error(`community event slice ${reduce.name} failed`, err, event);
        }
      }
      return;
    }
    if (reduceMembership(event)) return;
    if (reduceMessages(event)) return;
    reduceVoice(event);
  });
}

/// Community voice signalling, on the `voice-event` channel.
///
/// Separate from the community-event dispatcher because `VoiceEvent` is
/// one Tier 1 family and the backend routes all of it to `voice-event`.
/// The reducer filters to community-scoped calls; DM calls carry no
/// community channel state to update.
export function subscribeCommunityVoiceEvents(): Promise<UnlistenFn> {
  return subscribeVoiceEvents((event) => {
    reduceSubscriptionVoice({ voice: event });
  });
}
