import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeDeepLinkEvents } from "../ipc/channels";
import { handleJoinCommunity } from "./community.handlers";
import { addToast } from "../stores/toast.store";

export function subscribeDeepLinkHandler(): Promise<UnlistenFn> {
  return subscribeDeepLinkEvents(async (event) => {
    if (event.action === "joinCommunity") {
      addToast("Joining community via invite...", "info");
      // handleJoinCommunity shows success/error toasts internally and re-throws
      // on failure (for the modal path); there is no modal here, so swallow it.
      await handleJoinCommunity(
        event.communityId,
        "Invited community",
        event.inviteCode,
        event.secretsRecordKey,
      ).catch(() => {});
    }
  });
}
