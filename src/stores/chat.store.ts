import { createStore } from "solid-js/store";

export type MessageStatus = "sending" | "sent" | "queued" | "failed";

export interface ReactionGroup {
  emoji: string;
  count: number;
  reactors: string[];
}

export interface Message {
  id: number;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  replyTo?: number;
  status?: MessageStatus;
  serverMessageId?: string;
  replyToId?: string;
  editedAt?: number;
  reactions?: ReactionGroup[];
  pinned?: boolean;
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
