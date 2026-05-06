import type { UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "../ipc/commands";
import { subscribeChatEvents } from "../ipc/channels";
import { dmState, setDmState, type DmMessage } from "../stores/dm.store";
import { notificationState, setNotificationState } from "../stores/notification.store";

/// Subscribe to chat events for the DM subsystem (architecture §27).
/// Handles two flows:
///   1. `directMessageInvite` — surfaces invites in the pending list and
///      pushes a system notification.
///   2. `messageReceived` — when conversation_id matches an accepted DM
///      record key, append to that DM's message log.
export function subscribeDmInbox(getOwnPublicKey: () => string): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if (event.type === "directMessageInvite") {
      const { from, recordKey, initiatorPseudonym, isGroup } = event.data;
      setDmState("pendingInvites", recordKey, {
        recordKey,
        from,
        initiatorPseudonym,
        isGroup,
        receivedAt: Date.now(),
      });
      setNotificationState("notifications", (prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          type: "system",
          title: isGroup ? "Group DM invite" : "Direct message invite",
          body: `${initiatorPseudonym} wants to ${isGroup ? "add you to a group DM" : "start a direct message"}.`,
          timestamp: Date.now(),
          read: false,
        },
      ]);
      setNotificationState("unreadCount", (c) => c + 1);
      return;
    }

    if (event.type === "messageReceived") {
      const recordKey = event.data.conversationId;
      // Only handle conversation IDs that look like a DM record key (i.e.,
      // we have an accepted DM conversation tracked under that key).
      if (!dmState.conversations[recordKey]) return;
      const isOwn = event.data.from === getOwnPublicKey();
      const message: DmMessage = {
        id: Date.now(),
        senderId: event.data.from,
        body: event.data.body,
        timestamp: event.data.timestamp,
        isOwn,
      };
      const existing = dmState.messages[recordKey] ?? [];
      setDmState("messages", recordKey, [...existing, message]);
      setDmState("conversations", recordKey, "lastMessageAt", event.data.timestamp);
    }
  });
}

export async function handleListDms(): Promise<void> {
  const list = await commands.listDms();
  const map: Record<string, (typeof list)[number]> = {};
  for (const conv of list) {
    map[conv.recordKey] = conv;
  }
  setDmState("conversations", map);
}

export async function handleStartDm(
  bobPublicKey: string,
  alicePseudonym: string,
): Promise<string> {
  const recordKey = await commands.startDm(bobPublicKey, alicePseudonym);
  await handleListDms();
  setDmState("activeRecordKey", recordKey);
  await commands.openDmWindow(recordKey, alicePseudonym);
  return recordKey;
}

export async function handleAcceptDm(recordKey: string): Promise<void> {
  await commands.acceptDmInvite(recordKey);
  setDmState("pendingInvites", recordKey, undefined!);
  await handleListDms();
  setDmState("activeRecordKey", recordKey);
  const conv = dmState.conversations[recordKey];
  const title = conv?.initiatorPseudonym ?? recordKey.slice(0, 12);
  await commands.openDmWindow(recordKey, title);
}

export async function handleDeclineDm(recordKey: string): Promise<void> {
  await commands.declineDmInvite(recordKey);
  setDmState("pendingInvites", recordKey, undefined!);
}

export async function handleSendDm(recordKey: string, body: string): Promise<void> {
  await commands.sendDmMessage(recordKey, body);
  // The backend re-emits MessageReceived for the local write so the
  // subscribeDmInbox handler will append the message — no need to mutate
  // the store here; that keeps a single source of truth for ordering.
}

// Re-export for components that surface the current notification badge.
export { notificationState };
