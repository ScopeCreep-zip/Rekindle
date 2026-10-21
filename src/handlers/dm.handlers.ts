// Group DM event subscriptions (architecture §27.2), until E2.6's group DM
// events replace them. 1:1 DMs arrive through `chat-events.handlers.ts`.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { isLegacy } from "../ipc/channels/chat_events";
import { dmState, setDmState, type DmMessage } from "../stores/dm.store";

/// The buddy list's half: an incoming group DM invite goes to the pending
/// list (DmInviteModal). Its inbox row is the backend's `systemAlert`.
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
  });
}

/// The chat window's group mode: a message whose conversationId is an
/// accepted group DM's record key is appended to that group's log.
export function subscribeDmInbox(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // The legacy envelope is community channel messages.
    if (isLegacy(event) || !("channelMessage" in event)) return;
    const msg = event.channelMessage;
    if (!("directMessageReceived" in msg)) return;
    const dm = msg.directMessageReceived;
    const recordKey = dm.conversationId;
    if (!dmState.conversations[recordKey]) return;
    // `isOwn` is the backend's: the log's echo of our own send.
    const message: DmMessage = {
      id: Date.now(),
      senderId: dm.peerKey,
      body: dm.body ?? "",
      timestamp: dm.timestamp,
      isOwn: dm.isOwn,
      serverMessageId: dm.serverMessageId ?? undefined,
    };
    const existing = dmState.messages[recordKey] ?? [];
    // The backend's message id names the row: one already shown is not repeated.
    const idx = message.serverMessageId
      ? existing.findIndex((m) => m.serverMessageId === message.serverMessageId)
      : -1;
    if (idx >= 0) {
      setDmState("messages", recordKey, idx, (m) => ({ ...m, ...message, id: m.id }));
    } else {
      setDmState("messages", recordKey, [...existing, message]);
    }
    setDmState("conversations", recordKey, "lastMessageAt", dm.timestamp);
  });
}
