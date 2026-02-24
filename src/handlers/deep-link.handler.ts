import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeDeepLinkEvents } from "../ipc/channels";
import { handleJoinCommunity } from "./community.handlers";
import { communityState } from "../stores/community.store";
import { addToast } from "../stores/toast.store";

export function subscribeDeepLinkHandler(): Promise<UnlistenFn> {
  return subscribeDeepLinkEvents(async (event) => {
    if (event.action === "joinCommunity") {
      addToast("Joining community via invite...", "info");
      await handleJoinCommunity(event.communityId, "Invited community", event.inviteCode);
      // handleJoinCommunity catches errors internally and shows error toast,
      // so check if the community actually appeared in the store
      if (communityState.communities[event.communityId]) {
        addToast("Joined community!", "success");
      }
    }
  });
}
