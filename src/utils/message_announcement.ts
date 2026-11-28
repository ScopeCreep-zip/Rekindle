import type { Message } from "../stores/chat.store";

/**
 * What a screen reader says for one message (wiring guide §13b, §13g):
 * sender, time and body, with the removed and undecryptable cases named
 * instead of reading an empty body. The bubble's accessible name and the
 * chat window's new-message announcement both use it, so the two never
 * drift apart.
 */
export function messageAnnouncement(
  message: Message,
  senderName: string,
  removedByName: () => string
): string {
  const ts = new Date(message.timestamp).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  const sender = senderName || (message.isOwn ? "You" : "Unknown");
  const body = message.removedBy
    ? `Removed by ${removedByName()}`
    : message.undecryptable
      ? message.undecryptable.reasonText
      : message.decryptionFailed
        ? "Unable to decrypt — waiting for encryption keys"
        : message.body;
  return `${sender} at ${ts}: ${body}`;
}
