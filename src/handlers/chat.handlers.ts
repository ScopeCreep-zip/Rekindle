import { commands } from "../ipc/commands";
import { setChatState, chatState } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import { setFriendsState } from "../stores/friends.store";
import type { Message } from "../stores/chat.store";

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
    setChatState("conversations", to, "messages", (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "sent" as const } : m)),
    );
  } catch {
    // Update status to failed
    setChatState("conversations", to, "messages", (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "failed" as const } : m)),
    );
  }
}

let typingTimeout: ReturnType<typeof setTimeout> | null = null;
let isLocalTyping = false;

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
      handleSendMessage(to, body);
      clearInput();
      // Clear typing indicator on send
      if (isLocalTyping) {
        isLocalTyping = false;
        commands.sendTyping(to, false).catch(() => {});
      }
      if (typingTimeout) {
        clearTimeout(typingTimeout);
        typingTimeout = null;
      }
    }
  } else {
    // Send typing indicator on non-Enter key presses
    if (!isLocalTyping) {
      isLocalTyping = true;
      commands.sendTyping(to, true).catch(() => {});
    }
    // Reset the typing timeout — stop after 3s of no input
    if (typingTimeout) clearTimeout(typingTimeout);
    typingTimeout = setTimeout(() => {
      isLocalTyping = false;
      commands.sendTyping(to, false).catch(() => {});
      typingTimeout = null;
    }, 3000);
  }
}

export function handleIncomingMessage(
  peerId: string,
  message: Message,
): void {
  const existing = chatState.conversations[peerId];
  if (existing) {
    setChatState("conversations", peerId, "messages", (msgs) => [
      ...msgs,
      message,
    ]);
  } else {
    setChatState("conversations", peerId, {
      peerId,
      messages: [message],
      isTyping: false,
      lastRead: 0,
    });
  }
}

export function handleTypingIndicator(
  peerId: string,
  isTyping: boolean,
): void {
  if (chatState.conversations[peerId]) {
    setChatState("conversations", peerId, "isTyping", isTyping);
  }
}

export async function handleLoadHistory(
  peerId: string,
  limit: number,
): Promise<void> {
  try {
    const messages = await commands.getMessageHistory(peerId, limit);
    const mapped: Message[] = messages.map((m) => ({
      id: m.id,
      senderId: m.senderId,
      body: m.body,
      timestamp: m.timestamp,
      isOwn: m.isOwn,
    }));
    setChatState("conversations", peerId, {
      peerId,
      messages: mapped,
      isTyping: false,
      lastRead: 0,
    });
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
  setFriendsState("friends", peerId, "unreadCount", 0);
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
