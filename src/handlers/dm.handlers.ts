// Group DM event subscriptions (architecture §27.2), until E2.6's group DM
// events replace them. 1:1 DMs arrive through `chat-events.handlers.ts`.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { isLegacy } from "../ipc/channels/chat_events";
import { dmState, setDmState, type DmMessage } from "../stores/dm.store";
import { setNotificationState } from "../stores/notification.store";

/// The buddy list's half: an incoming group DM invite goes to the pending
/// list (DmInviteModal) and the notification inbox.
export function subscribeDmInvites(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if (isLegacy(event) || !("channelMessage" in event)) return;
    const msg = event.channelMessage;
    if (!("directConversationInvited" in msg)) return;
    const { from, recordKey, initiatorPseudonym } = msg.directConversationInvited;
    setDmState("pendingInvites", recordKey, {
      recordKey,
      from,
      initiatorPseudonym,
      receivedAt: Date.now(),
    });
    setNotificationState("notifications", (prev) => [
      ...prev,
      {
        id: crypto.randomUUID(),
        type: "system",
        title: "Group DM invite",
        body: `${initiatorPseudonym} wants to add you to a group DM.`,
        timestamp: Date.now(),
        read: false,
      },
    ]);
    setNotificationState("unreadCount", (c) => c + 1);
  });
}

/// The chat window's group mode: a message whose conversationId is an
/// accepted group DM's record key is appended to that group's log.
export function subscribeDmInbox(getOwnPublicKey: () => string): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // The legacy envelope is community channel messages.
    if (isLegacy(event) || !("channelMessage" in event)) return;
    const msg = event.channelMessage;
    if (!("directMessageReceived" in msg)) return;
    const dm = msg.directMessageReceived;
    const recordKey = dm.conversationId;
    if (!dmState.conversations[recordKey]) return;
    const message: DmMessage = {
      id: Date.now(),
      senderId: dm.peerKey,
      body: dm.body ?? "",
      timestamp: dm.timestamp,
      isOwn: dm.peerKey === getOwnPublicKey(),
    };
    const existing = dmState.messages[recordKey] ?? [];
    setDmState("messages", recordKey, [...existing, message]);
    setDmState("conversations", recordKey, "lastMessageAt", dm.timestamp);
  });
}
