import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

import type { Message as IpcMessage } from "../ipc/commands/types";

export type MessageStatus = "sending" | "sent" | "queued" | "failed";

export interface ReactionGroup {
  emoji: string;
  count: number;
  reactors: string[];
}

export interface PollAnswer {
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
export interface Message extends IpcMessage {
  /** Delivery state, tracked locally — never sent by the backend. */
  status?: MessageStatus;
  replyTo?: number;
  replyToId?: string;
  editedAt?: number;
  /** Named form of the inherited inline shape. */
  reactions?: ReactionGroup[];
  poll?: MessagePoll;
  attachment?: MessageAttachment;
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
  /** Set after a download completes locally; absent until then.
   *  UI flips "Download" → "Open" when this is non-null. */
  localPath?: string | null;
}

export interface Conversation {
  peerId: string;
  messages: Message[];
  isTyping: boolean;
  lastRead: number;
}

export interface ChatState {
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
export interface BulkSelectionState {
  channelId: string;
  selectedIds: Set<string>;
}

const [bulkSelection, setBulkSelection] = createSignal<BulkSelectionState | null>(null);

export { bulkSelection, setBulkSelection };

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
