import { createStore } from "solid-js/store";

export type MessageStatus = "sending" | "sent" | "failed";

export interface Message {
  id: number;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  replyTo?: number;
  status?: MessageStatus;
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
