import { createEffect, createRoot, untrack } from "solid-js";
import { communityState } from "../../stores/community.store";

/**
 * Run `apply` once `community`'s row is in this window's store: at once
 * when it is held, else when the Host's arrival event (or hydration) lists
 * it. An action that issued a create or join waits here instead of adding
 * the row from its own reply (lead decision (41): one path for rows).
 */
export function onCommunityListed(community: string, apply: () => void): void {
  if (communityState.communities[community]) {
    apply();
    return;
  }
  createRoot((dispose) => {
    createEffect(() => {
      if (!communityState.communities[community]) return;
      dispose();
      untrack(apply);
    });
  });
}
