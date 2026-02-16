import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { setNotificationState } from "../stores/notification.store";
import { communityState, setCommunityState } from "../stores/community.store";
import { handleTypingIndicator, handleIncomingMessage, handleResetUnread } from "./chat.handlers";
import { handleRefreshFriends } from "./buddy.handlers";
import type { Message } from "../stores/chat.store";

export function subscribeBuddyListChatEvents(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    switch (event.type) {
      case "messageReceived": {
        const senderId = event.data.from;
        if (friendsState.friends[senderId]) {
          setFriendsState("friends", senderId, "unreadCount", (c) => (c ?? 0) + 1);
        }
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
        handleRefreshFriends();
        break;
      }
      case "friendAdded": {
        setFriendsState("friends", event.data.publicKey, {
          publicKey: event.data.publicKey,
          displayName: event.data.displayName,
          nickname: null,
          status: "offline" as const,
          statusMessage: null,
          gameInfo: null,
          group: "Friends",
          unreadCount: 0,
          lastSeenAt: null,
          voiceChannel: null,
        });
        break;
      }
      case "friendRequestRejected": {
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
        if (event.data.from === getOwnKey()) break;
        if (event.data.conversationId === peerId) {
          handleIncomingMessage(peerId, {
            id: Date.now(),
            senderId: event.data.from,
            body: event.data.body,
            timestamp: event.data.timestamp,
            isOwn: false,
          });
          handleResetUnread(peerId);
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
        timestamp: event.data.timestamp,
        isOwn: false,
      };
      const existing = communityState.channelMessages[channelId];
      if (existing) {
        setCommunityState("channelMessages", channelId, (msgs) => [
          ...msgs,
          message,
        ]);
      } else {
        setCommunityState("channelMessages", channelId, [message]);
      }
    } else if (event.type === "channelHistoryLoaded") {
      const { channelId, messages: serverMsgs } = event.data;
      const existing = communityState.channelMessages[channelId] ?? [];
      const existingKeys = new Set(
        existing.map((m) => `${m.timestamp}:${m.senderId}`),
      );
      const newMsgs: Message[] = serverMsgs
        .filter((m) => !existingKeys.has(`${m.timestamp}:${m.senderId}`))
        .map((m) => ({
          id: m.id,
          senderId: m.senderId,
          body: m.body,
          timestamp: m.timestamp,
          isOwn: m.isOwn,
        }));
      if (newMsgs.length > 0) {
        const merged = [...existing, ...newMsgs].sort(
          (a, b) => a.timestamp - b.timestamp,
        );
        setCommunityState("channelMessages", channelId, merged);
      }
    }
  });
}
