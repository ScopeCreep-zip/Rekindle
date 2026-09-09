import { batch } from "solid-js";
import { reconcile } from "solid-js/store";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { isLegacy } from "../ipc/channels/chat_events";
import type { FriendEvent } from "../ipc/channels/chat_events";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { setNotificationState } from "../stores/notification.store";
import { communityState, setCommunityState } from "../stores/community.store";
import { chatState, setChatState } from "../stores/chat.store";
import { handleTypingIndicator, handleIncomingMessage, handleResetUnread } from "../actions/chat.actions";
import { handleRefreshFriends } from "../actions/buddy.actions";
import type { Message } from "../stores/chat.store";
import { transformNewFriend } from "../utils/transformers";

// Architecture §16 — flip the optimistic "sending"/"queued" status
// on the matching outbound DM to "sent". The Rust ack carries the
// SQLite-row timestamp (ms); the optimistic frontend used a slightly
// earlier `Date.now()` so we accept a small fuzz window. The matching
// pass scans all conversations because a single ack doesn't carry
// the peer id (the message_id is unique enough on its own).
function applyMessageAck(messageId: number): void {
  const FUZZ_MS = 5000;
  for (const peerId in chatState.conversations) {
    const convo = chatState.conversations[peerId];
    if (!convo) continue;
    const idx = convo.messages.findIndex(
      (m) =>
        m.isOwn &&
        (m.status === "sending" || m.status === "queued") &&
        Math.abs(m.timestamp - messageId) <= FUZZ_MS,
    );
    if (idx >= 0) {
      setChatState(
        "conversations",
        peerId,
        "messages",
        idx,
        "status",
        "sent" as const,
      );
      return;
    }
  }
}

export function subscribeBuddyListChatEvents(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // Legacy envelope: community channel messages only.
    if (isLegacy(event)) {
      batch(() => {
        const senderId = event.data.from;
        if (friendsState.friends[senderId]) {
          setFriendsState("friends", senderId, "unreadCount", (c) => (c ?? 0) + 1);
        }
      });
      return;
    }

    if ("channelMessage" in event) {
      const msg = event.channelMessage;
      if ("directMessageReceived" in msg) {
        const senderId = msg.directMessageReceived.peerKey;
        batch(() => {
          if (friendsState.friends[senderId]) {
            setFriendsState("friends", senderId, "unreadCount", (c) => (c ?? 0) + 1);
          }
        });
      } else if ("directMessageAcknowledged" in msg) {
        applyMessageAck(msg.directMessageAcknowledged.messageId);
      }
      return;
    }

    if ("typing" in event) {
      const t = event.typing;
      const started = "started" in t;
      const inner = started ? t.started : t.stopped;
      if ("dm" in inner.context) {
        handleTypingIndicator(inner.context.dm.peerKey, started);
      }
      return;
    }

    if ("friend" in event) {
      applyFriendEvent(event.friend);
    }
  });
}

/** Apply one friend-lifecycle event to the friends and notification stores. */
function applyFriendEvent(friend: FriendEvent): void {
  if ("requestReceived" in friend) {
    const { fromKey, displayName, message } = friend.requestReceived;
    // Update display name if sender is already a friend (bidirectional add)
    if (friendsState.friends[fromKey]) {
      setFriendsState("friends", fromKey, "displayName", displayName);
    }
    const exists = friendsState.pendingRequests.some((r) => r.publicKey === fromKey);
    if (!exists) {
      setFriendsState("pendingRequests", (reqs) => [
        ...reqs,
        { publicKey: fromKey, displayName, message },
      ]);
    }
    return;
  }

  if ("accepted" in friend) {
    // Use reconcile to force SolidJS to diff and fire all changed signals.
    // Plain nested setters (setStore("friends", key, "prop", val)) can miss
    // memo recomputation when the memo iterates via Object.values().
    const { peerKey, displayName } = friend.accepted;
    const existing = friendsState.friends[peerKey];
    if (existing) {
      setFriendsState(
        "friends",
        peerKey,
        reconcile({
          ...existing,
          friendshipState: "accepted" as const,
          displayName: displayName || existing.displayName,
        }),
      );
    }
    handleRefreshFriends();
    return;
  }

  if ("added" in friend) {
    const { peerKey, displayName, friendshipState } = friend.added;
    setFriendsState(
      "friends",
      peerKey,
      transformNewFriend(peerKey, displayName, friendshipState),
    );
    return;
  }

  if ("rejected" in friend) {
    // Remove the pending-out friend from the list
    const { peerKey } = friend.rejected;
    if (friendsState.friends[peerKey]) {
      const next = { ...friendsState.friends };
      delete next[peerKey];
      setFriendsState("friends", reconcile(next));
    }
    const truncatedKey = peerKey.slice(0, 8);
    setNotificationState("notifications", (prev) => [
      ...prev,
      {
        id: crypto.randomUUID(),
        type: "system",
        title: "Friend Request Declined",
        body: `Your friend request was declined by ${truncatedKey}...`,
        timestamp: Date.now(),
        read: false,
      },
    ]);
    setNotificationState("unreadCount", (c) => c + 1);
    return;
  }

  if ("removed" in friend) {
    const next = { ...friendsState.friends };
    delete next[friend.removed.peerKey];
    setFriendsState("friends", reconcile(next));
  }
  // requestAcknowledged / removeAcknowledged / profileKeyRotated need no
  // store change: delivery receipts and key rotation are handled by the
  // backend, and the list already reflects the outcome.
}

export function subscribeDmChatEvents(
  peerId: string,
  getOwnKey: () => string,
): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // DMs arrive on the daemon vocabulary; the legacy envelope is
    // community channel messages only, which this window ignores.
    if (isLegacy(event)) return;

    if ("channelMessage" in event && "directMessageReceived" in event.channelMessage) {
      const dm = event.channelMessage.directMessageReceived;
      if (dm.peerKey === getOwnKey()) return;
      if (dm.conversationId !== peerId) return;
      batch(() => {
        handleIncomingMessage(peerId, {
          id: Date.now(),
          senderId: dm.peerKey,
          body: dm.body ?? "",
          decryptionFailed: dm.decryptionFailed,
          automodBlurred: dm.automodBlurred,
          timestamp: dm.timestamp,
          isOwn: false,
        });
        handleResetUnread(peerId);
      });
      return;
    }

    if ("typing" in event) {
      const t = event.typing;
      const started = "started" in t;
      const inner = started ? t.started : t.stopped;
      if ("dm" in inner.context && inner.context.dm.peerKey === peerId) {
        handleTypingIndicator(peerId, started);
      }
    }
  });
}

export function subscribeCommunityChannelChatEvents(
  getMyPseudonymKey: () => string | null | undefined,
): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if (isLegacy(event)) {
      const myPseudo = getMyPseudonymKey();
      if (myPseudo && event.data.from === myPseudo) return;
      const channelId = event.data.conversationId;
      const message: Message = {
        id: Date.now(),
        senderId: event.data.from,
        body: event.data.body,
        decryptionFailed: event.data.decryptionFailed,
        automodBlurred: event.data.automodBlurred,
        timestamp: event.data.timestamp,
        isOwn: false,
        serverMessageId: event.data.serverMessageId,
        replyToId: event.data.replyToId,
      };

      // If the backend resolved a display name, ensure it's in the community member list
      // so the memberNames memo picks it up (fixes names showing as public keys).
      if (event.data.senderDisplayName) {
        for (const [communityId, community] of Object.entries(communityState.communities)) {
          const hasChannel = community.channels.some((ch) => ch.id === channelId);
          if (hasChannel) {
            const existingIdx = community.members.findIndex(
              (m) => m.pseudonymKey === event.data.from,
            );
            if (existingIdx < 0) {
              // Member not yet in list — add a minimal entry
              setCommunityState("communities", communityId, "members", (prev) => [
                ...prev,
                {
                  pseudonymKey: event.data.from,
                  displayName: event.data.senderDisplayName!,
                  roleIds: [],
                  displayRole: "member",
                  status: "online" as const,
                  timeoutUntil: null,
                  gameInfo: null,
                },
              ]);
            } else if (community.members[existingIdx].displayName !== event.data.senderDisplayName) {
              // Update display name if changed
              setCommunityState(
                "communities", communityId, "members", existingIdx,
                "displayName", event.data.senderDisplayName!,
              );
            }
            break;
          }
        }
      }
      const existing = communityState.channelMessages[channelId];
      if (existing) {
        setCommunityState("channelMessages", channelId, (msgs) => [
          ...msgs,
          message,
        ]);
      } else {
        setCommunityState("channelMessages", channelId, [message]);
      }

      // Increment unread count if the channel is NOT the currently active one
      if (channelId !== communityState.activeChannel) {
        // Find which community this channel belongs to
        for (const [communityId, community] of Object.entries(communityState.communities)) {
          const chIdx = community.channels.findIndex((ch) => ch.id === channelId);
          if (chIdx >= 0) {
            setCommunityState("communities", communityId, "channels", chIdx, "unreadCount",
              (prev: number) => (prev ?? 0) + 1,
            );
            break;
          }
        }
      }
    }
  });
}
