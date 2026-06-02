import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeCommunityEvents } from "../../ipc/channels";
import { reduceMembership } from "./dispatcher_members";
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
    if (reduceMembership(event)) return;
    if (reduceMessages(event)) return;
    if (reduceVoice(event)) return;
    reduceContent(event);
  });
}
