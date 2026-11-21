/**
 * Co-creation (plan E3.2 S7s.3f): the co-creator invites friends hold for
 * us, and each co-creator's state in a creation of ours. Fed only by the
 * Host's friend events (`creatorInvited`, `creatorInviteWithdrawn`,
 * `coCreatorStatus`); the window decides nothing, it renders what the Host
 * said. The creation the Create Community dialog is watching is the one
 * piece of UI state here: which anchor its status view shows.
 */

import { createStore, produce } from "solid-js/store";
import type { FriendEvent } from "../ipc/channels/chat_events";

/** A friend's invite to co-create `communityName`, held until answered. */
export interface CreatorInvite {
  anchor: string;
  peerKey: string;
  communityName: string;
}

interface CoCreatorState {
  /** Held invites by anchor. */
  invites: Record<string, CreatorInvite>;
  /** Per creation anchor, each co-creator's state as the Host last stated
   *  it (`pending`, `accepted`, `declined`, `removed`, `withdrawn`,
   *  `joining`). Kept by anchor because the Host states `pending` before
   *  the create call returns. */
  statuses: Record<string, Record<string, string>>;
  /** The creation the status view shows, and the name it was given. */
  watching: { anchor: string; name: string } | null;
}

const [coCreatorState, setCoCreatorState] = createStore<CoCreatorState>({
  invites: {},
  statuses: {},
  watching: null,
});

/** Apply a co-creation friend event; whether `friend` was one. */
export function applyCoCreatorEvent(friend: FriendEvent): boolean {
  if ("creatorInvited" in friend) {
    const { peerKey, anchor, communityName } = friend.creatorInvited;
    setCoCreatorState("invites", anchor, { anchor, peerKey, communityName });
    return true;
  }
  if ("creatorInviteWithdrawn" in friend) {
    const { peerKey, anchor } = friend.creatorInviteWithdrawn;
    if (coCreatorState.invites[anchor]?.peerKey === peerKey) dropCreatorInvite(anchor);
    return true;
  }
  if ("coCreatorStatus" in friend) {
    const { peerKey, anchor, status } = friend.coCreatorStatus;
    setCoCreatorState(
      "statuses",
      produce((all) => {
        all[anchor] = { ...(all[anchor] ?? {}), [peerKey]: status };
      }),
    );
    return true;
  }
  return false;
}

/** Forget a held invite: answered, or withdrawn by its inviter. */
export function dropCreatorInvite(anchor: string): void {
  setCoCreatorState(
    "invites",
    produce((all) => {
      delete all[anchor];
    }),
  );
}

/** Show `anchor`'s creation in the status view, seeding the co-creators
 *  the Host listed as pending that no event has stated yet. */
export function watchCreation(anchor: string, name: string, pending: string[]): void {
  setCoCreatorState(
    produce((s) => {
      const known = s.statuses[anchor] ?? {};
      for (const peer of pending) known[peer] ??= "pending";
      s.statuses[anchor] = known;
      s.watching = { anchor, name };
    }),
  );
}

/** Close the status view and drop its creation's states. */
export function stopWatchingCreation(): void {
  setCoCreatorState(
    produce((s) => {
      if (s.watching) delete s.statuses[s.watching.anchor];
      s.watching = null;
    }),
  );
}

export { coCreatorState };
