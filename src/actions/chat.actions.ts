import { commands } from "../ipc/commands";
import { setChatState, chatState } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import { friendsState, setFriendsState } from "../stores/friends.store";
import type { Message } from "../stores/chat.store";
import { transformMessages } from "../utils/transformers";

export async function handleSendMessage(to: string, body: string): Promise<void> {
  const trimmed = body.trim();
  if (!trimmed) return;

  const tempId = Date.now();

  // Optimistic insert with "sending" status
  const message: Message = {
    id: tempId,
    senderId: authState.publicKey ?? "",
    body: trimmed,
    timestamp: Date.now(),
    isOwn: true,
    status: "sending",
  };
  handleIncomingMessage(to, message);

  try {
    await commands.sendMessage(to, trimmed);
    // Update status to sent
    const convo = chatState.conversations[to];
    if (convo) {
      setChatState("conversations", to, {
        ...convo,
        messages: convo.messages.map((m) => (m.id === tempId ? { ...m, status: "sent" as const } : m)),
      });
    }
  } catch {
    // Update status to failed
    const convo = chatState.conversations[to];
    if (convo) {
      setChatState("conversations", to, {
        ...convo,
        messages: convo.messages.map((m) => (m.id === tempId ? { ...m, status: "failed" as const } : m)),
      });
    }
  }
}

export function handleKeyDown(
  e: KeyboardEvent,
  to: string,
  getBody: () => string,
  clearInput: () => void,
): void {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    const body = getBody();
    if (body.trim()) {
      // The backend ends our typing when the message goes out.
      handleSendMessage(to, body);
      clearInput();
    }
  }
}

/// Report the 1:1 draft to `to` after each edit: whether it holds text.
/// The backend decides what the peer sees (the start, its refresh, the
/// idle stop; plan E2.3), so the window keeps no typing state. Typing is
/// best effort, so a failed report is dropped.
export function handleDraftChanged(to: string, body: string): void {
  commands.sendTyping(to, body.trim().length > 0).catch(() => {});
}

export function handleIncomingMessage(
  peerId: string,
  message: Message,
): void {
  const existing = chatState.conversations[peerId];
  if (existing) {
    setChatState("conversations", peerId, {
      ...existing,
      messages: [...existing.messages, message],
    });
  } else {
    setChatState("conversations", peerId, {
      peerId,
      messages: [message],
      isTyping: false,
      lastRead: 0,
    });
  }
}

/// Render the peer's typing as the backend emitted it: the start, and the
/// stop on their stop, their message, or the backend's expiry (plan E2.3).
export function handleTypingIndicator(
  peerId: string,
  isTyping: boolean,
): void {
  const existing = chatState.conversations[peerId];
  if (existing) {
    setChatState("conversations", peerId, {
      ...existing,
      isTyping,
    });
  }
}

export async function handleLoadHistory(
  peerId: string,
  limit: number,
): Promise<void> {
  try {
    const messages = await commands.getMessageHistory(peerId, limit);
    const mapped = transformMessages(messages);
    const existing = chatState.conversations[peerId];
    if (mapped.length > 0 || !existing || existing.messages.length === 0) {
      setChatState("conversations", peerId, {
        peerId,
        messages: mapped,
        isTyping: existing?.isTyping ?? false,
        lastRead: existing?.lastRead ?? 0,
      });
    }
  } catch (e) {
    console.error("Failed to load history:", e);
  }
}

export function handleMarkRead(peerId: string): void {
  commands.markRead(peerId).catch((e) => {
    console.error("Failed to mark read:", e);
  });
}

export function handleResetUnread(peerId: string): void {
  if (friendsState.friends[peerId]) {
    setFriendsState("friends", peerId, "unreadCount", 0);
  }
  commands.markRead(peerId).catch((e) => {
    console.error("Failed to mark read:", e);
  });
}

export async function handleRetrySendMessage(peerId: string, messageId: number): Promise<void> {
  const convo = chatState.conversations[peerId];
  if (!convo) return;
  const message = convo.messages.find((m) => m.id === messageId);
  if (!message || message.status !== "failed") return;

  // Set status back to sending
  setChatState("conversations", peerId, "messages", (msgs) =>
    msgs.map((m) => (m.id === messageId ? { ...m, status: "sending" as const } : m)),
  );

  try {
    await commands.sendMessage(peerId, message.body);
    setChatState("conversations", peerId, "messages", (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status: "sent" as const } : m)),
    );
  } catch {
    setChatState("conversations", peerId, "messages", (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status: "failed" as const } : m)),
    );
  }
}
