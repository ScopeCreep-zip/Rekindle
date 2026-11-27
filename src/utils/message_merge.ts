import type { Message } from "../stores/chat.store";

/**
 * A history read joined with the list it lands on (plan CP1). The read is
 * the Host's stored rows when it began, so a row it cannot have seen stays:
 * one of ours still waiting for its id, or one newer than the read's newest
 * (an echo or a post that arrived while the read was in flight). A row
 * inside the read's window that the read lacks was removed, and goes.
 */
export function mergeRead(existing: Message[], read: Message[]): Message[] {
  const ids = new Set(read.map((m) => m.serverMessageId).filter((id) => id !== undefined));
  const newest = read.reduce((t, m) => Math.max(t, m.timestamp), Number.NEGATIVE_INFINITY);
  const later = existing.filter(
    (m) =>
      !(m.serverMessageId !== undefined && ids.has(m.serverMessageId)) &&
      (m.serverMessageId === undefined || m.timestamp > newest)
  );
  return [...read, ...later];
}
