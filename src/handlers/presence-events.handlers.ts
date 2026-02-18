import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribePresenceEvents } from "../ipc/channels";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { communityState, setCommunityState } from "../stores/community.store";
import { authState, setAuthState } from "../stores/auth.store";
import type { UserStatus } from "../stores/auth.store";

export function subscribeBuddyListPresenceEvents(): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    switch (event.type) {
      case "friendOnline": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "status", "online");
        }
        break;
      }
      case "friendOffline": {
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState("friends", event.data.publicKey, "status", "offline");
          setFriendsState("friends", event.data.publicKey, "lastSeenAt", Date.now());
        }
        break;
      }
      case "statusChanged": {
        // Sync own status when auto-away changes it from the backend
        if (event.data.publicKey === authState.publicKey) {
          setAuthState("status", event.data.status as UserStatus);
        }
        if (friendsState.friends[event.data.publicKey]) {
          setFriendsState(
            "friends",
            event.data.publicKey,
            "status",
            event.data.status as UserStatus,
          );
          if (event.data.statusMessage !== undefined) {
            setFriendsState(
              "friends",
              event.data.publicKey,
              "statusMessage",
              event.data.statusMessage,
            );
          }
        }
        break;
      }
      case "gameChanged": {
        if (friendsState.friends[event.data.publicKey]) {
          if (event.data.gameName) {
            setFriendsState("friends", event.data.publicKey, "gameInfo", {
              gameName: event.data.gameName,
              gameId: event.data.gameId,
              startedAt: event.data.elapsedSeconds,
            });
          } else {
            setFriendsState("friends", event.data.publicKey, "gameInfo", null);
          }
        }
        break;
      }
    }
  });
}

export function subscribeChatPresenceEvents(
  peerId: string,
  setPeerStatus: (s: UserStatus) => void,
): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    switch (event.type) {
      case "friendOnline": {
        if (event.data.publicKey === peerId) setPeerStatus("online");
        break;
      }
      case "friendOffline": {
        if (event.data.publicKey === peerId) setPeerStatus("offline");
        break;
      }
      case "statusChanged": {
        if (event.data.publicKey === peerId) {
          setPeerStatus(event.data.status as UserStatus);
        }
        break;
      }
    }
  });
}

export function subscribeCommunityPresenceEvents(): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    const key =
      event.type === "friendOnline" || event.type === "friendOffline"
        ? event.data.publicKey
        : event.type === "statusChanged"
          ? event.data.publicKey
          : null;
    if (!key) return;
    const newStatus =
      event.type === "friendOnline"
        ? "online"
        : event.type === "friendOffline"
          ? "offline"
          : event.type === "statusChanged"
            ? event.data.status
            : null;
    if (!newStatus) return;
    for (const communityId of Object.keys(communityState.communities)) {
      const community = communityState.communities[communityId];
      const memberIdx = community.members.findIndex((m) => m.pseudonymKey === key);
      if (memberIdx >= 0) {
        setCommunityState("communities", communityId, "members", memberIdx, "status", newStatus);
      }
    }
  });
}

export function subscribeProfilePresenceEvents(
  publicKey: string,
): Promise<UnlistenFn> {
  return subscribePresenceEvents((event) => {
    switch (event.type) {
      case "friendOnline": {
        if (event.data.publicKey === publicKey && friendsState.friends[publicKey]) {
          setFriendsState("friends", publicKey, "status", "online");
        }
        break;
      }
      case "friendOffline": {
        if (event.data.publicKey === publicKey && friendsState.friends[publicKey]) {
          setFriendsState("friends", publicKey, "status", "offline");
          setFriendsState("friends", publicKey, "lastSeenAt", Date.now());
        }
        break;
      }
      case "statusChanged": {
        if (event.data.publicKey === publicKey && friendsState.friends[publicKey]) {
          setFriendsState("friends", publicKey, "status", event.data.status as UserStatus);
          if (event.data.statusMessage !== undefined) {
            setFriendsState("friends", publicKey, "statusMessage", event.data.statusMessage);
          }
        }
        break;
      }
      case "gameChanged": {
        if (event.data.publicKey === publicKey && friendsState.friends[publicKey]) {
          if (event.data.gameName) {
            setFriendsState("friends", publicKey, "gameInfo", {
              gameName: event.data.gameName,
              gameId: event.data.gameId,
              startedAt: event.data.elapsedSeconds,
            });
          } else {
            setFriendsState("friends", publicKey, "gameInfo", null);
          }
        }
        break;
      }
    }
  });
}
