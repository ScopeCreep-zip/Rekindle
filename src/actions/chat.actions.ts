import { commands } from "../ipc/commands";
import { setChatState, chatState } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import { friendsState, setFriendsState } from "../stores/friends.store";
import type { Message } from "../stores/chat.store";
import { transformDmMessage } from "../utils/transformers";

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
    const sent = transformDmMessage(await commands.sendMessage(to, trimmed));
    setChatState("conversations", to, "messages", (msgs) =>
      msgs.map((m) => (m.id === tempId ? sent : m)),
    );
  } catch {
    // The Host stored the row as failed: show it as the Host holds it, so
    // its Retry names the stored message.
    await handleLoadHistory(to, 50);
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
    // The backend's message id names the row: a message already shown
    // (our own send, echoed back) is updated in place, not repeated.
    const idx = message.serverMessageId
      ? existing.messages.findIndex((m) => m.serverMessageId === message.serverMessageId)
      : -1;
    if (idx >= 0) {
      setChatState("conversations", peerId, "messages", idx, (m) => ({ ...m, ...message, id: m.id }));
      return;
    }
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
    const mapped = messages.map(transformDmMessage);
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

/// Mark the conversation read; the Host publishes the zero count
/// (`UnreadChanged`), which every window renders.
export function handleResetUnread(peerId: string): void {
  commands.markRead(peerId).catch((e) => {
    console.error("Failed to mark read:", e);
  });
}

/// Retry our failed message: the Host re-sends the stored row by its
/// client id (`DmResend`); nothing is re-sent as a new message.
export async function handleRetrySendMessage(peerId: string, messageId: number): Promise<void> {
  const convo = chatState.conversations[peerId];
  const message = convo?.messages.find((m) => m.id === messageId);
  const clientId = message?.serverMessageId;
  if (!message || message.status !== "failed" || !clientId) return;
  const setStatus = (status: Message["status"]) =>
    setChatState("conversations", peerId, "messages", (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status } : m)),
    );
  setStatus("sending");
  try {
    setStatus(transformDmMessage(await commands.resendMessage(clientId)).status);
  } catch {
    setStatus("failed");
  }
}
