import type { Message } from "../stores/chat.store";

/**
 * A history read joined with the list it lands on (plan CP1). The read is
 * the Host's stored rows when it began, so a row it cannot have seen stays:
 * one of ours still waiting for its id, or one newer than the read's newest
 * (an echo or a post that arrived while the read was in flight). A row
 * older than the read's oldest is history the read did not reach (earlier
 * scrollback), and stays before it. Only a row inside the read's window
 * that the read lacks was removed, and goes.
 */
export function mergeRead(existing: Message[], read: Message[]): Message[] {
  // An empty read has no window: nothing it holds replaces or removes a row
  // (with no oldest or newest, every row would land both before and after it).
  if (read.length === 0) return existing;
  const ids = new Set(read.map((m) => m.serverMessageId).filter((id) => id !== undefined));
  const newest = read.reduce((t, m) => Math.max(t, m.timestamp), Number.NEGATIVE_INFINITY);
  const oldest = read.reduce((t, m) => Math.min(t, m.timestamp), Number.POSITIVE_INFINITY);
  const unread = existing.filter(
    (m) => !(m.serverMessageId !== undefined && ids.has(m.serverMessageId))
  );
  const earlier = unread.filter((m) => m.serverMessageId !== undefined && m.timestamp < oldest);
  const later = unread.filter((m) => m.serverMessageId === undefined || m.timestamp > newest);
  return [...earlier, ...read, ...later];
}
