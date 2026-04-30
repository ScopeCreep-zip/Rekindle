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
