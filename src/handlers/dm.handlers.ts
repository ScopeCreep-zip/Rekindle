// DM inbox event subscription. The user actions that used to sit
// beside it now live in `src/actions/dm.actions.ts`.

import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { isLegacy } from "../ipc/channels/chat_events";
import { dmState, setDmState, type DmMessage } from "../stores/dm.store";
import { setNotificationState } from "../stores/notification.store";

/// Subscribe to chat events for the DM subsystem (architecture §27).
/// Handles two flows:
///   1. `directConversationInvited` — surfaces invites in the pending
///      list and pushes a system notification.
///   2. `directMessageReceived` — when conversationId matches an
///      accepted DM record key, append to that DM's message log.
export function subscribeDmInbox(getOwnPublicKey: () => string): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // The legacy envelope is community channel messages, which the DM
    // inbox does not track.
    if (isLegacy(event)) return;
    if (!("channelMessage" in event)) return;
    const msg = event.channelMessage;

    if ("directConversationInvited" in msg) {
      const { from, recordKey, initiatorPseudonym, isGroup } =
        msg.directConversationInvited;
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

    if ("directMessageReceived" in msg) {
      const dm = msg.directMessageReceived;
      const recordKey = dm.conversationId;
      // Only handle conversation IDs that look like a DM record key (i.e.,
      // we have an accepted DM conversation tracked under that key).
      if (!dmState.conversations[recordKey]) return;
      const isOwn = dm.peerKey === getOwnPublicKey();
      const message: DmMessage = {
        id: Date.now(),
        senderId: dm.peerKey,
        body: dm.body ?? "",
        timestamp: dm.timestamp,
        isOwn,
      };
      const existing = dmState.messages[recordKey] ?? [];
      setDmState("messages", recordKey, [...existing, message]);
      setDmState("conversations", recordKey, "lastMessageAt", dm.timestamp);
    }
  });
}
