import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";

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

export interface Message {
  id: number;
  senderId: string;
  body: string;
  decryptionFailed?: boolean;
  automodBlurred?: boolean;
  timestamp: number;
  isOwn: boolean;
  replyTo?: number;
  status?: MessageStatus;
  serverMessageId?: string;
  replyToId?: string;
  editedAt?: number;
  reactions?: ReactionGroup[];
  pinned?: boolean;
  poll?: MessagePoll;
  /** Pseudonym (hex) of the original author when this row originated from a Forward.
   *  `null`/undefined for native messages. Backend populates from SQLite column
   *  `forwarded_from_author` written by `services/community/channel_messages::forward_message`. */
  forwardedFromAuthor?: string | null;
  /** Lost Cargo attachment metadata (architecture §28.9). Decoded from
   *  the SQLite `attachment_json` column populated by upload + receive paths. */
  attachment?: MessageAttachment;
  /** Bitfield from `ChannelEntry::Message.flags` — VOICE_MESSAGE=0x10 (architecture §16.4),
   *  SUPPRESS_NOTIFICATIONS=0x20, etc. */
  flags?: number;
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
