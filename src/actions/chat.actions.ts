import { commands } from "../ipc/commands";
import { setChatState, chatState } from "../stores/chat.store";
import { authState } from "../stores/auth.store";
import { friendsState, setFriendsState } from "../stores/friends.store";
import type { Message } from "../stores/chat.store";
import { transformDmMessage } from "../utils/transformers";
import { awaitsExpiry } from "./disappearing.actions";
import { afterDraftReports, sendDraftReport } from "../utils/draft_reports";
import { withOwnRow } from "../utils/own_rows";

/**
 * Send `body` to `to`. Nothing is drawn here: once the send commits, the
 * Host publishes our own row (`directMessageSent`, plan E6.11) to this
 * window and every other that shows the conversation, and a window opened
 * later replays it from the journal; the command's answer is that same row,
 * upserted alike. A failed send's row is the Host's stored one, read back so
 * its Retry names the stored message.
 */
export async function handleSendMessage(to: string, body: string): Promise<void> {
  const trimmed = body.trim();
  if (!trimmed) return;
  try {
    upsertOwnMessage(to, transformDmMessage(await afterDraftReports(`dm:${to}`, () => commands.sendMessage(to, trimmed))));
  } catch {
    await handleLoadHistory(to, 50);
  }
}

/**
 * Our own committed 1:1 row into the conversation `to`, by its stored
 * message id (`serverMessageId`): the row holding it is replaced, else it
 * joins in time order. The Host's event and the send's answer both land
 * here, in either order, and leave one row.
 */
export function upsertOwnMessage(to: string, message: Message): void {
  const existing = chatState.conversations[to];
  if (!existing) {
    setChatState("conversations", to, { peerId: to, messages: [message], isTyping: false, lastRead: 0 });
    return;
  }
  setChatState("conversations", to, "messages", (msgs) => withOwnRow(msgs, message));
}

/// Whether a draft can be sent: it holds more than whitespace. The Send
/// button is disabled while this is false, and Enter sends nothing.
export function canSend(body: string): boolean {
  return body.trim().length > 0;
}

/// Send the 1:1 draft to `to` and clear the input, when it can be sent
/// (Enter and the Send button both land here).
export function sendDraft(to: string, getBody: () => string, clearInput: () => void): void {
  const body = getBody();
  if (canSend(body)) {
    // The backend ends our typing when the message goes out.
    handleSendMessage(to, body);
    clearInput();
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
    sendDraft(to, getBody, clearInput);
  }
}

/// Report the 1:1 draft to `to` after each edit: whether it holds text.
/// The backend decides what the peer sees (the start, its refresh, the
/// idle stop; plan E2.3), so the window keeps no typing state. Typing is
/// best effort, so a failed report is dropped.
/** Our 1:1 composer changed: whether it holds text, in edit order and never
 *  swallowed (utils/draft_reports.ts). */
export function handleDraftChanged(to: string, body: string): void {
  void sendDraftReport(`dm:${to}`, () => commands.sendTyping(to, body.trim().length > 0));
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
    // Rows that arrived live after the history was read stay (peer finding
    // F-echo): a row the snapshot lacks and newer than its newest row cannot
    // be one the snapshot dropped (a page of the newest `limit`), and
    // replacing the list lost our own sent row when the read began before
    // its commit and answered after the Host's event.
    const newest = mapped.reduce((t, m) => Math.max(t, m.timestamp), 0);
    const stored = new Set(mapped.map((m) => m.serverMessageId).filter(Boolean));
    const live = (existing?.messages ?? []).filter(
      (m) => m.serverMessageId && !stored.has(m.serverMessageId) && m.timestamp > newest,
    );
    if (mapped.length > 0 || live.length > 0 || !existing || existing.messages.length === 0) {
      setChatState("conversations", peerId, {
        peerId,
        messages: [...mapped, ...live],
        isTyping: existing?.isTyping ?? false,
        lastRead: existing?.lastRead ?? 0,
      });
    }
  } catch (e) {
    console.error("Failed to load history:", e);
  }
}

export function handleMarkRead(peerId: string): void {
  handleResetUnread(peerId);
}

/// The client id of the newest of the peer's messages this window has
/// rendered while it has focus, or null: the Host sends our read marker
/// for it when our receipts setting is on (plan E2.5).
function renderedThrough(peerId: string): string | null {
  if (!document.hasFocus()) return null;
  const messages = chatState.conversations[peerId]?.messages ?? [];
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (!m.isOwn && m.serverMessageId) return m.serverMessageId;
  }
  return null;
}

/// Mark the conversation read once the frame with its messages is
/// painted; the Host publishes the zero count (`UnreadChanged`), which
/// every window renders, and starts the disappearing timers of what was
/// displayed (plan E7.1): their expiry is then read from the stored rows.
export function handleResetUnread(peerId: string): void {
  requestAnimationFrame(() => {
    commands
      .markRead(peerId, renderedThrough(peerId))
      .then(() => {
        const messages = chatState.conversations[peerId]?.messages ?? [];
        if (awaitsExpiry(messages)) void handleLoadHistory(peerId, Math.max(50, messages.length));
      })
      .catch((e) => {
        console.error("Failed to mark read:", e);
      });
  });
}

/// Retry our failed message: the Host queues the stored row again under
/// its client id (`OutboxRetry`); its state arrives as `DmSendStatus`.
export async function handleRetrySendMessage(peerId: string, messageId: number): Promise<void> {
  const clientId = failedClientId(peerId, messageId);
  if (!clientId) return;
  await commands.outboxRetry(clientId).catch((e) => {
    console.error("Retry failed:", e);
  });
}

/// Discard our failed message (`OutboxDiscard`, reviewed by the Host's
/// intent gate): it stays failed and the messages behind it go on.
export async function handleDiscardSendMessage(peerId: string, messageId: number): Promise<void> {
  const clientId = failedClientId(peerId, messageId);
  if (!clientId) return;
  await commands.outboxDiscard(clientId).catch((e) => {
    console.error("Discard failed:", e);
  });
}

function failedClientId(peerId: string, messageId: number): string | undefined {
  const message = chatState.conversations[peerId]?.messages.find((m) => m.id === messageId);
  return message?.status === "failed" ? message.serverMessageId : undefined;
}
