import type { Message } from "../stores/chat.store";

/**
 * Our own committed row into a conversation's list, by its stored message
 * id (plan E6.11, the own-row events and the send's answer): the row holding
 * that id is replaced, else the new row joins in time order.
 *
 * A held row keeps its delivery state (`status`, `retry`): the new row is
 * the commit-time snapshot (`queued`), and a held row's state is that same
 * snapshot or newer, since every later state arrives only as a send-status
 * event (`dmSendStatus`, group `sendStatus`). Replacing it put a sent
 * message back on the pending clock whenever the send's answer arrived after
 * the outbox reported it sent, and it stayed there until a receipt (Linux,
 * 2026-11-23: "Alice's own DM stays on the pending clock"). A retry's states
 * come through the same events, so they still move it.
 */
export function withOwnRow(messages: Message[], row: Message): Message[] {
  const held = row.serverMessageId
    ? messages.find((m) => m.serverMessageId === row.serverMessageId)
    : undefined;
  const next: Message = held ? { ...row, status: held.status, retry: held.retry } : row;
  const rest = held ? messages.filter((m) => m !== held) : messages;
  return [...rest, next].sort((x, y) => x.timestamp - y.timestamp);
}
