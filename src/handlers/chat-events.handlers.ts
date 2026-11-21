import { batch } from "solid-js";
import { reconcile } from "solid-js/store";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeChatEvents } from "../ipc/channels";
import { conversationId } from "../ipc/channels/chat_events";
import { applyTimerChanged, withoutExpired } from "../actions/disappearing.actions";
import type { DmHistoryGap, DmSendStatus, FriendEvent } from "../ipc/channels/chat_events";
import { friendsState, setFriendsState } from "../stores/friends.store";
import { applyCoCreatorEvent } from "../stores/co-creator.store";
import { chatState, setChatState } from "../stores/chat.store";
import { handleTypingIndicator, handleIncomingMessage, handleResetUnread } from "../actions/chat.actions";
import { handleRefreshFriends } from "../actions/buddy.actions";
import { transformHistoryGap, transformNewFriend } from "../utils/transformers";

// A DM we sent changed delivery state (`dmSendStatus`): the
// message whose client id (`serverMessageId`) it names takes the state the
// backend names. No matching or inference happens here.
function applyMessageState(state: DmSendStatus): void {
  if (state.clientId.startsWith("FriendRequest:")) {
    if (friendsState.friends[state.conversationId]) {
      setFriendsState("friends", state.conversationId, "requestSendState", state.state);
    }
    return;
  }
  const convo = chatState.conversations[state.conversationId];
  if (!convo) return;
  const idx = convo.messages.findIndex((m) => m.serverMessageId === state.clientId);
  if (idx < 0) return;
  setChatState("conversations", state.conversationId, "messages", idx, "status", state.state);
}

// The peer's stream lost entries we never fetched (`dmHistoryGap`): the
// backend stored a marker row; it is appended as history would show it,
// once per `throughSeq`. The backend decides there is a gap; nothing here
// infers one.
function applyHistoryGap(gap: DmHistoryGap): void {
  const marker = transformHistoryGap(gap);
  const convo = chatState.conversations[gap.peerKey];
  if (!convo) {
    setChatState("conversations", gap.peerKey, {
      peerId: gap.peerKey,
      messages: [marker],
      isTyping: false,
      lastRead: 0,
    });
    return;
  }
  if (convo.messages.some((m) => m.historyGapThrough === gap.throughSeq)) return;
  setChatState("conversations", gap.peerKey, "messages", (msgs) => [...msgs, marker]);
}

export function subscribeBuddyListChatEvents(): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    // A DM's unread count is the Host's (`UnreadChanged`), rendered as is.
    if ("unreadChanged" in event) {
      const { context, count } = event.unreadChanged;
      if (typeof context === "object" && "dm" in context && friendsState.friends[context.dm.peerKey]) {
        setFriendsState("friends", context.dm.peerKey, "unreadCount", count);
      }
      return;
    }

    if ("channelMessage" in event) {
      const msg = event.channelMessage;
      if ("dmSendStatus" in msg) {
        applyMessageState(msg.dmSendStatus);
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

/** The peer a friend-lifecycle event changes the friendship with. */
function friendEventPeer(friend: FriendEvent): string | null {
  if ("accepted" in friend) return friend.accepted.peerKey;
  if ("added" in friend) return friend.added.peerKey;
  if ("rejected" in friend) return friend.rejected.peerKey;
  if ("removed" in friend) return friend.removed.peerKey;
  return null;
}

/** Apply one friend-lifecycle event to the friends and notification stores. */
function applyFriendEvent(friend: FriendEvent): void {
  // Co-creation invites and statuses (plan E3.2 S7s.3f) have their own store.
  if (applyCoCreatorEvent(friend)) return;
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
          handshakeError: null,
          handshakeRetrying: false,
          unreachable: false,
        }),
      );
    }
    handleRefreshFriends();
    return;
  }

  if ("added" in friend) {
    const { peerKey, displayName, friendshipState, groupLabel } = friend.added;
    setFriendsState(
      "friends",
      peerKey,
      transformNewFriend(peerKey, displayName, friendshipState, groupLabel),
    );
    // The Host's view of the row (its group-DM eligibility, GC3).
    handleRefreshFriends();
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
    // Its inbox row is the backend's systemAlert.
    return;
  }

  if ("removed" in friend) {
    const next = { ...friendsState.friends };
    delete next[friend.removed.peerKey];
    setFriendsState("friends", reconcile(next));
    return;
  }

  if ("handshakeFailed" in friend) {
    const { peerKey, reason } = friend.handshakeFailed;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "handshakeError", reason);
      setFriendsState("friends", peerKey, "handshakeRetrying", false);
    }
    // Our accept publishes `accepted` before its session opens (GC3), so
    // the re-read that `accepted` started can predate this failure; a newer
    // read (newest wins) carries the Host's row with the error.
    handleRefreshFriends();
    return;
  }
  if ("identityPinMismatch" in friend) {
    const { peerKey } = friend.identityPinMismatch;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "identityPinMismatch", true);
    }
    return;
  }
  if ("identityPinRestored" in friend) {
    const { peerKey } = friend.identityPinRestored;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "identityPinMismatch", false);
    }
    return;
  }
  if ("blockChanged" in friend) {
    // A block also removes the friend (`removed`); an unblock adds no row,
    // so the list is re-read from the Host.
    if (!friend.blockChanged.blocked) handleRefreshFriends();
    return;
  }
  if ("reachability" in friend) {
    // The backend's route liveness verdict (plan RH1r-c S4).
    const { peerKey, reachable } = friend.reachability;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "unreachable", !reachable);
    }
    return;
  }
  if ("handshakeRetrying" in friend) {
    const { peerKey } = friend.handshakeRetrying;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "handshakeRetrying", true);
    }
    return;
  }

  if ("nicknameChanged" in friend) {
    const { peerKey, nickname } = friend.nicknameChanged;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "nickname", nickname);
    }
    return;
  }
  if ("displayNameChanged" in friend) {
    const { peerKey, displayName } = friend.displayNameChanged;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, "displayName", displayName);
    }
    return;
  }
  if ("groupChanged" in friend) {
    const { peerKey, groupId, groupLabel } = friend.groupChanged;
    if (friendsState.friends[peerKey]) {
      setFriendsState("friends", peerKey, { group: groupLabel, groupId });
    }
    return;
  }
  if ("keyChanged" in friend) {
    // The friend rotated their identity: the Host moved them to the new
    // key, unverified (plan E2.3).
    const { peerKey, newKey } = friend.keyChanged;
    const moved = friendsState.friends[peerKey];
    if (moved) {
      const next = { ...friendsState.friends, [newKey]: { ...moved, publicKey: newKey } };
      delete next[peerKey];
      setFriendsState("friends", reconcile(next));
    }
    // Their conversation follows them; the shell reopens their windows
    // on the new key (audit R-5).
    const conversation = chatState.conversations[peerKey];
    if (conversation) {
      const next = { ...chatState.conversations, [newKey]: { ...conversation, peerId: newKey } };
      delete next[peerKey];
      setChatState("conversations", reconcile(next));
    }
    return;
  }
  // verificationChanged is rendered by each window's `useSafetyNumber`;
  // requestAcknowledged / removeAcknowledged / profileKeyRotated need no
  // store change: delivery receipts and key rotation are handled by the
  // backend, and the list already reflects the outcome.
}

/// DM events for one chat window. `onIncoming` runs after an incoming
/// message has been added to the store.
export function subscribeDmChatEvents(
  peerId: string,
  getOwnKey: () => string,
  onIncoming: () => void,
): Promise<UnlistenFn> {
  return subscribeChatEvents((event) => {
    if ("channelMessage" in event && "directMessageReceived" in event.channelMessage) {
      const dm = event.channelMessage.directMessageReceived;
      if (dm.peerKey === getOwnKey()) return;
      if (dm.conversationId !== peerId) return;
      batch(() => {
        // `isOwn`: we sent it, from another of our devices (E5.3).
        handleIncomingMessage(peerId, {
          id: Date.now(),
          senderId: dm.isOwn ? getOwnKey() : dm.peerKey,
          body: dm.body ?? "",
          decryptionFailed: dm.decryptionFailed,
          automodBlurred: dm.automodBlurred,
          timestamp: dm.timestamp,
          isOwn: dm.isOwn,
          serverMessageId: dm.serverMessageId ?? undefined,
          replyToId: dm.replyToId ?? undefined,
        });
        handleResetUnread(peerId);
      });
      onIncoming();
      return;
    }

    // Our own message changed delivery state: render the state the
    // backend names in this window too (the buddy list's arm does the same).
    if ("channelMessage" in event && "dmSendStatus" in event.channelMessage) {
      const state = event.channelMessage.dmSendStatus;
      if (state.conversationId === peerId) applyMessageState(state);
      return;
    }

    // The conversation's disappearing timer changed (plan E7.1): the
    // control takes it and the timeline shows who set it to what.
    if ("channelMessage" in event && "disappearingTimerChanged" in event.channelMessage) {
      const change = event.channelMessage.disappearingTimerChanged;
      if (conversationId(change.context) !== peerId) return;
      const notice = applyTimerChanged(change, getOwnKey());
      handleIncomingMessage(peerId, notice);
      onIncoming();
      return;
    }

    // The Host deleted expired messages (plan E7.1): drop them here.
    if ("channelMessage" in event && "messagesExpired" in event.channelMessage) {
      const gone = event.channelMessage.messagesExpired;
      if (conversationId(gone.context) !== peerId || !chatState.conversations[peerId]) return;
      setChatState("conversations", peerId, "messages", (msgs) => withoutExpired(msgs, gone.clientIds));
      onIncoming();
      return;
    }

    // Their stream to us lost entries (plan E2.5): draw the gap marker.
    if ("channelMessage" in event && "dmHistoryGap" in event.channelMessage) {
      const gap = event.channelMessage.dmHistoryGap;
      if (gap.peerKey === peerId) applyHistoryGap(gap);
      return;
    }

    // This friend's lifecycle (accepted, added, rejected, removed): the
    // window renders the Host's friendship state, so a pending request's
    // notice clears when their accept lands (L-UI1).
    if ("friend" in event && friendEventPeer(event.friend) === peerId) {
      applyFriendEvent(event.friend);
      return;
    }

    // Their keys are not signed by the identity we know: the input refuses
    // to send (plan E2.4, B3).
    if ("friend" in event && "identityPinMismatch" in event.friend) {
      if (event.friend.identityPinMismatch.peerKey === peerId && friendsState.friends[peerId]) {
        setFriendsState("friends", peerId, "identityPinMismatch", true);
      }
      return;
    }
    if ("friend" in event && "identityPinRestored" in event.friend) {
      if (event.friend.identityPinRestored.peerKey === peerId && friendsState.friends[peerId]) {
        setFriendsState("friends", peerId, "identityPinMismatch", false);
      }
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
