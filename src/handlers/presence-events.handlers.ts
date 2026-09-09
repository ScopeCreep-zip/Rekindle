import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribePresenceEvents } from "../ipc/channels";
import type {
  PresenceEvent,
  PresenceSnapshot,
} from "../ipc/channels/presence_events";
import { gameName } from "../ipc/channels/presence_events";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { communityState, setCommunityState } from "../stores/community.store";
import { authState, setAuthState } from "../stores/auth.store";
import type { UserStatus } from "../stores/auth.store";
import { transformGameInfo } from "../utils/transformers";

/**
 * Who the event is about, and what was observed about them.
 *
 * The four old event kinds (friendOnline / friendOffline / statusChanged
 * / gameChanged) collapsed into three subjects plus one snapshot, so
 * every handler below asks the same two questions instead of switching
 * on a kind and then re-deriving the key from a differently-named field.
 */
function subject(event: PresenceEvent): {
  key: string;
  snapshot: PresenceSnapshot;
  isSelf: boolean;
} {
  if ("friendChanged" in event) {
    return {
      key: event.friendChanged.peerKey,
      snapshot: event.friendChanged.snapshot,
      isSelf: false,
    };
  }
  if ("selfChanged" in event) {
    return {
      key: event.selfChanged.publicKey,
      snapshot: event.selfChanged.snapshot,
      isSelf: true,
    };
  }
  return {
    key: event.communityMemberChanged.pseudonym,
    snapshot: event.communityMemberChanged.snapshot,
    isSelf: false,
  };
}

/**
 * Apply the observed fields of a snapshot to one friend row.
 *
 * Only what was observed: `status: null` means the emitter never looked
 * at the status, so leaving it alone is what stops a game update from
 * knocking a friend offline (and a status update from clearing a game).
 */
function applyToFriend(publicKey: string, snapshot: PresenceSnapshot): void {
  if (!friendsState.friends[publicKey]) return;

  if (snapshot.status !== null) {
    setFriendsState("friends", publicKey, "status", snapshot.status as UserStatus);
    if (snapshot.status === "offline") {
      setFriendsState("friends", publicKey, "lastSeenAt", Date.now());
    }
  }
  if (snapshot.statusMessage !== null) {
    setFriendsState("friends", publicKey, "statusMessage", snapshot.statusMessage);
  }
  if (snapshot.game !== null) {
    const name = gameName(snapshot);
    if (name === null) {
      setFriendsState("friends", publicKey, "gameInfo", null);
    } else {
      const playing = snapshot.game === "idle" ? null : snapshot.game.playing;
      setFriendsState(
        "friends",
        publicKey,
        "gameInfo",
        transformGameInfo({
          gameName: name,
          gameId: playing?.gameId ?? null,
          elapsedSeconds: playing?.elapsedSeconds ?? null,
          serverAddress: playing?.serverAddress ?? null,
        }),
      );
    }
  }
}

export function subscribeBuddyListPresenceEvents(): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    const { key, snapshot, isSelf } = subject(event);

    // Sync own status when auto-away changes it from the backend.
    if ((isSelf || key === authState.publicKey) && snapshot.status !== null) {
      setAuthState("status", snapshot.status as UserStatus);
    }
    if (!isSelf) applyToFriend(key, snapshot);
  });
}

export function subscribeChatPresenceEvents(
  peerId: string,
  setPeerStatus: (s: UserStatus) => void,
): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    const { key, snapshot, isSelf } = subject(event);
    if (isSelf || key !== peerId || snapshot.status === null) return;
    setPeerStatus(snapshot.status as UserStatus);
  });
}

export function subscribeCommunityPresenceEvents(): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    const { key, snapshot, isSelf } = subject(event);
    if (isSelf || snapshot.status === null) return;

    // A community member event names its own community, so only that
    // one needs scanning. A friend event carries no community, so every
    // community is searched for a member with that key — the same peer
    // can be both a friend and a co-member.
    const communityIds =
      "communityMemberChanged" in event
        ? [event.communityMemberChanged.community]
        : Object.keys(communityState.communities);

    for (const communityId of communityIds) {
      const community = communityState.communities[communityId];
      if (!community) continue;
      const memberIdx = community.members.findIndex((m) => m.pseudonymKey === key);
      if (memberIdx >= 0) {
        setCommunityState(
          "communities",
          communityId,
          "members",
          memberIdx,
          "status",
          snapshot.status,
        );
      }
    }
  });
}

export function subscribeProfilePresenceEvents(
  publicKey: string,
): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    const { key, snapshot, isSelf } = subject(event);
    if (isSelf || key !== publicKey) return;
    applyToFriend(publicKey, snapshot);
  });
}
