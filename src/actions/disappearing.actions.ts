import { commands } from "../ipc/commands";
import { isCancelled } from "../ipc/commands/dto";
import type { DisappearingTimerChanged } from "../ipc/channels/chat_events";
import { conversationId } from "../ipc/channels/chat_events";
import type { Message } from "../stores/chat.store";
import { disappearingState, setDisappearingState } from "../stores/disappearing.store";
import { errorMessage } from "../utils/error";

// Disappearing messages (plan E7.1). The Host holds the timer, its presets,
// each message's expiry and the deletion; the window reads the timer, asks
// the Host to change it (reviewed: `SetDisappearingTimer`), and renders the
// events.

/** Read the timer of `conversation` (a friend's key or a group's id). */
export async function handleLoadDisappearingTimer(conversation: string): Promise<void> {
  try {
    const view = await commands.disappearingGet(conversation);
    setDisappearingState("timers", conversation, view);
  } catch (e) {
    console.error("disappearing_get failed", e);
  }
}

/** Ask the Host to set the timer; the control follows the
 *  `disappearingTimerChanged` the Host then publishes. `null` on success
 *  (or a cancelled review), else the Host's error. */
export async function handleSetDisappearingTimer(
  conversation: string,
  seconds: number,
): Promise<string | null> {
  try {
    await commands.disappearingSet(conversation, seconds);
    return null;
  } catch (e) {
    return isCancelled(e) ? null : errorMessage(e);
  }
}

let noticeRows = 0;

/** Apply a timer change to the control and return its timeline notice. */
export function applyTimerChanged(change: DisappearingTimerChanged, ownKey: string): Message {
  const id = conversationId(change.context);
  const held = disappearingState.timers[id];
  if (held) setDisappearingState("timers", id, "seconds", change.seconds);
  noticeRows += 1;
  return {
    // Local rows below any live-message or gap-marker id.
    id: -2_000_000_000 - noticeRows,
    senderId: change.setBy,
    body: "",
    timestamp: Date.now(),
    isOwn: change.setBy === ownKey,
    timerNotice: { seconds: change.seconds, setBy: change.setBy },
  };
}

/** `messages` without the ones the Host deleted. */
export function withoutExpired(messages: Message[], clientIds: string[]): Message[] {
  const gone = new Set(clientIds);
  return messages.filter((m) => !m.serverMessageId || !gone.has(m.serverMessageId));
}

/** A message whose expiry this window has not read: a live copy (its row
 *  is not loaded, so its timer is unknown) or a received one whose timer
 *  had not started. Once the Host starts timers at display, the window
 *  re-reads the rows. */
export function awaitsExpiry(messages: Message[]): boolean {
  return messages.some(
    (m) =>
      m.serverMessageId !== undefined &&
      m.historyGapThrough === undefined &&
      (m.expireSeconds === undefined ||
        (!m.isOwn && m.expireSeconds > 0 && m.expireAt === undefined)),
  );
}

/** Reloaded rows with the window's timer notices kept in time order: a
 *  notice is the event's, not a stored row. */
export function withNotices(loaded: Message[], held: Message[]): Message[] {
  const notices = held.filter((m) => m.timerNotice !== undefined);
  if (notices.length === 0) return loaded;
  return [...loaded, ...notices].sort((a, b) => a.timestamp - b.timestamp);
}
