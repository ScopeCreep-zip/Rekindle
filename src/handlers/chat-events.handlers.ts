import { batch } from "solid-js";
import { reconcile } from "solid-js/store";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { setNotificationState } from "../stores/notification.store";
import { communityState, setCommunityState } from "../stores/community.store";
import { chatState, setChatState } from "../stores/chat.store";
import { handleTypingIndicator, handleIncomingMessage, handleResetUnread } from "./chat.handlers";
import { handleRefreshFriends } from "./buddy.handlers";
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
    switch (event.type) {
      case "messageReceived": {
        batch(() => {
          const senderId = event.data.from;
          if (friendsState.friends[senderId]) {
            setFriendsState("friends", senderId, "unreadCount", (c) => (c ?? 0) + 1);
          }
        });
        break;
      }
      case "typingIndicator": {
        handleTypingIndicator(event.data.from, event.data.typing);
        break;
      }
      case "friendRequest": {
        // Update display name if sender is already a friend (bidirectional add)
        if (friendsState.friends[event.data.from]) {
          setFriendsState("friends", event.data.from, "displayName", event.data.displayName);
        }
        const exists = friendsState.pendingRequests.some(
          (r) => r.publicKey === event.data.from,
        );
        if (!exists) {
          setFriendsState("pendingRequests", (reqs) => [
            ...reqs,
            {
              publicKey: event.data.from,
              displayName: event.data.displayName,
              message: event.data.message,
            },
          ]);
        }
        break;
      }
      case "friendRequestAccepted": {
        // Use reconcile to force SolidJS to diff and fire all changed signals.
        // Plain nested setters (setStore("friends", key, "prop", val)) can miss
        // memo recomputation when the memo iterates via Object.values().
        const accepted = friendsState.friends[event.data.from];
        if (accepted) {
          setFriendsState("friends", event.data.from, reconcile({
            ...accepted,
            friendshipState: "accepted" as const,
            displayName: event.data.displayName || accepted.displayName,
          }));
        }
        handleRefreshFriends();
        break;
      }
      case "friendAdded": {
        setFriendsState("friends", event.data.publicKey,
          transformNewFriend(event.data.publicKey, event.data.displayName, event.data.friendshipState),
        );
        break;
      }
      case "friendRequestRejected": {
        // Remove the pending-out friend from the list
        if (friendsState.friends[event.data.from]) {
          const next = { ...friendsState.friends };
          delete next[event.data.from];
          setFriendsState("friends", reconcile(next));
        }
        const truncatedKey = event.data.from.slice(0, 8);
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
        break;
      }
      case "friendRemoved": {
        const next = { ...friendsState.friends };
        delete next[event.data.publicKey];
        setFriendsState("friends", reconcile(next));
        break;
      }
      case "friendRequestDelivered": {
        // Optional: could show a delivery indicator on the pending friend
        break;
      }
      case "messageAck": {
        applyMessageAck(event.data.messageId);
        break;
      }
    }
  });
}

export function subscribeDmChatEvents(
  peerId: string,
  getOwnKey: () => string,
): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    switch (event.type) {
      case "messageReceived": {
        console.warn("[DM] messageReceived event:", event.data.conversationId, "peerId:", peerId);
        if (event.data.from === getOwnKey()) break;
        if (event.data.conversationId === peerId) {
          batch(() => {
            handleIncomingMessage(peerId, {
              id: Date.now(),
              senderId: event.data.from,
              body: event.data.body,
              decryptionFailed: event.data.decryptionFailed,
              automodBlurred: event.data.automodBlurred,
              timestamp: event.data.timestamp,
              isOwn: false,
            });
            handleResetUnread(peerId);
          });
        }
        break;
      }
      case "typingIndicator": {
        if (event.data.from === peerId) {
          handleTypingIndicator(peerId, event.data.typing);
        }
        break;
      }
    }
  });
}

export function subscribeCommunityChannelChatEvents(
  getMyPseudonymKey: () => string | null | undefined,
): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if (event.type === "messageReceived") {
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
