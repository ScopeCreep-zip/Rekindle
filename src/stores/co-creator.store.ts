/**
 * Co-creation (plan E3.2 S7s.3f): the co-creator invites friends hold for
 * us, and each co-creator's state in a creation of ours. Fed only by the
 * Host's friend events (`creatorInvited`, `creatorInviteWithdrawn`,
 * `coCreatorStatus`); the window decides nothing, it renders what the Host
 * said. The creation the Create Community dialog is watching is the one
 * piece of UI state here: which anchor its status view shows. A window
 * opened after the unlock replay reads the Host's snapshot once its event
 * stream is live (plan E3.2 S7s.3g, VI1's pattern); an event that arrives
 * while the snapshot is in flight wins over it.
 */

import { createStore, produce } from "solid-js/store";
import type { FriendEvent } from "../ipc/channels/chat_events";
import type { CoCreationSnapshot } from "../ipc/commands/community";

/** A friend's invite to co-create `communityName`, held until answered. */
interface CreatorInvite {
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
  /** Our creations the Host said ended: `true` written, `false` failed. */
  finished: Record<string, boolean>;
  /** The creation the status view shows, and the name it was given. */
  watching: { anchor: string; name: string } | null;
}

const [coCreatorState, setCoCreatorState] = createStore<CoCreatorState>({
  invites: {},
  statuses: {},
  finished: {},
  watching: null,
});

/**
 * When each piece of state was last set by an event (`i:` invite, `s:`
 * co-creator status, `f:` finish), on one counter: each event carries its
 * piece's whole state and the stream delivers in order, so a piece an
 * event set while the snapshot was in flight is never older than the
 * snapshot's, and the snapshot leaves it alone (plan VI1).
 */
let seq = 0;
const setBy = new Map<string, number>();
function touch(key: string): void {
  seq += 1;
  setBy.set(key, seq);
}

/** Apply a co-creation friend event; whether `friend` was one. */
export function applyCoCreatorEvent(friend: FriendEvent): boolean {
  if ("creatorInvited" in friend) {
    const { peerKey, anchor, communityName } = friend.creatorInvited;
    touch(`i:${anchor}`);
    setCoCreatorState("invites", anchor, { anchor, peerKey, communityName });
    return true;
  }
  if ("creatorInviteWithdrawn" in friend) {
    const { peerKey, anchor } = friend.creatorInviteWithdrawn;
    touch(`i:${anchor}`);
    if (coCreatorState.invites[anchor]?.peerKey === peerKey) dropCreatorInvite(anchor);
    return true;
  }
  if ("coCreatorStatus" in friend) {
    const { peerKey, anchor, status } = friend.coCreatorStatus;
    touch(`s:${anchor}:${peerKey}`);
    setCoCreatorState(
      "statuses",
      produce((all) => {
        all[anchor] = { ...(all[anchor] ?? {}), [peerKey]: status };
      }),
    );
    return true;
  }
  if ("coCreationFinished" in friend) {
    const { anchor, created } = friend.coCreationFinished;
    touch(`f:${anchor}`);
    setCoCreatorState("finished", anchor, created);
    return true;
  }
  return false;
}

/** The event sequence number now; a snapshot read after it yields to any
 *  state an event set since. */
export function coCreationSeq(): number {
  return seq;
}

/** Apply the Host's co-creation snapshot read after `before`: a state an
 *  event set since then is the newer statement and is kept. */
export function applyCoCreationSnapshot(snapshot: CoCreationSnapshot, before: number): void {
  const fresh = (key: string): boolean => (setBy.get(key) ?? 0) <= before;
  setCoCreatorState(
    produce((s) => {
      for (const { anchor, peerKey, communityName } of snapshot.invites) {
        if (fresh(`i:${anchor}`)) s.invites[anchor] = { anchor, peerKey, communityName };
      }
      for (const creation of snapshot.creations) {
        if (!fresh(`f:${creation.anchor}`)) continue;
        const known = s.statuses[creation.anchor] ?? {};
        for (const { peerKey, status } of creation.coCreators) {
          if (fresh(`s:${creation.anchor}:${peerKey}`)) known[peerKey] = status;
        }
        s.statuses[creation.anchor] = known;
        s.watching ??= { anchor: creation.anchor, name: creation.name };
      }
    }),
  );
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
      if (s.watching) {
        delete s.statuses[s.watching.anchor];
        delete s.finished[s.watching.anchor];
      }
      s.watching = null;
    }),
  );
}

export { coCreatorState };
