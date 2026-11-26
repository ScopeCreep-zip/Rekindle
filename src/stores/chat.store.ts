import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

import type { Message as IpcMessage } from "../ipc/commands/types";
import type { DeliveryRetry } from "../ipc/channels/chat_events";

type MessageStatus =
  | "sending"
  | "awaiting_key"
  | "queued"
  | "sent"
  | "delivered"
  | "read"
  | "failed";

export interface ReactionGroup {
  emoji: string;
  count: number;
  reactors: string[];
}

interface PollAnswer {
  index: number;
  text: string;
  voteCount: number;
  voters: string[];
}

export interface MessagePoll {
  pollId: string;
  question: string;
  answers: PollAnswer[];
  multiSelect: boolean;
  expiresAt?: number;
  closed: boolean;
  selectedAnswers: number[];
}

/**
 * A chat message as the UI holds it: the IPC wire shape plus the
 * client-side fields the backend does not send.
 *
 * This used to redeclare every wire field, which drifted from
 * `ipc/commands/types.ts` in both directions — that copy lacked
 * `replyTo`/`replyToId`/`editedAt`, this one lacked nothing but
 * duplicated a dozen fields. Extending keeps the shared shape in one
 * place without having to decide, field by field, which side owns it.
 */
export interface Message extends Omit<IpcMessage, "sendState"> {
  /** Delivery state: from the backend's `sendState` when a message is
   *  loaded (our own only), then updated locally as a send resolves. */
  status?: MessageStatus;
  /** Our send goes unanswered and the Host keeps trying (plan SR2), else null. */
  retry?: DeliveryRetry | null;
  /** Why our failed post was not sent, as the backend words it (a key that
   *  never came, or written after our removal), live or from the stored
   *  row; absent when the network superseded it. */
  failureReason?: string;
  replyTo?: number;
  replyToId?: string;
  editedAt?: number;
  /** Named form of the inherited inline shape. */
  reactions?: ReactionGroup[];
  poll?: MessagePoll;
  attachment?: MessageAttachment;
  /** A DM history-gap marker (plan E2.5, D23): the peer's messages through
   *  this seq are gone. The row is the backend's, drawn as a gap line in
   *  place of a bubble. */
  historyGapThrough?: number;
  /** The disappearing timer it carries (plan E7.1; 0 = none), from the
   *  stored row; absent on a live copy until a reload reads the row. */
  expireSeconds?: number;
  /** When the Host deletes it (ms), from the stored row. */
  expireAt?: number;
  /** A timeline notice, not a message (plan E7.1): `setBy` set the
   *  conversation's disappearing timer to `seconds` (0 = off). */
  timerNotice?: { seconds: number; setBy: string };
}

/** VOICE_MESSAGE bit on `Message.flags` per architecture §16.4. */
export const FLAG_VOICE_MESSAGE = 0x10;
export const FLAG_SUPPRESS_NOTIFICATIONS = 0x20;

export interface MessageAttachment {
  attachmentId: string;
  filename: string;
  mimeType: string;
  totalSize: number;
  chunkCount: number;
  /** The file was saved to disk on this device ("Show in folder"). */
  downloaded: boolean;
}

interface Conversation {
  peerId: string;
  messages: Message[];
  isTyping: boolean;
  lastRead: number;
}

interface ChatState {
  conversations: Record<string, Conversation>;
  activeConversation: string | null;
}

const [chatState, setChatState] = createStore<ChatState>({
  conversations: {},
  activeConversation: null,
});

export { chatState, setChatState };

/**
 * Multi-select state for the channel currently in admin bulk-delete mode.
 * Only one channel can be in selection mode at a time. Set to null to exit.
 *
 * `selectedIds` keys are `serverMessageId` strings ("msg_<uuid>"). The local
 * SQLite numeric `id` is not used — server ids are what flow through governance.
 */
interface BulkSelectionState {
  channelId: string;
  selectedIds: Set<string>;
}

const [bulkSelection, setBulkSelection] = createSignal<BulkSelectionState | null>(null);

export { bulkSelection };

export function toggleBulkSelected(messageId: string): void {
  const current = bulkSelection();
  if (!current) return;
  const next = new Set(current.selectedIds);
  if (next.has(messageId)) {
    next.delete(messageId);
  } else {
    next.add(messageId);
  }
  setBulkSelection({ channelId: current.channelId, selectedIds: next });
}

export function startBulkSelection(channelId: string): void {
  setBulkSelection({ channelId, selectedIds: new Set() });
}

export function clearBulkSelection(): void {
  setBulkSelection(null);
}
