import { createStore } from "solid-js/store";
import type { DmConversation } from "../ipc/commands";

// Group DMs over SMPL records (architecture §27.2), until E2.6's group DM
// store replaces this. 1:1 DMs are the friend's chat (`chat.store.ts`).

export interface DmInviteRequest {
  recordKey: string;
  from: string;
  initiatorPseudonym: string;
  receivedAt: number;
}

export interface DmMessage {
  id: number;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
  /** The backend's id for the message, when it names one. */
  serverMessageId?: string;
}

export interface DmState {
  /** Group invites awaiting accept/decline. */
  pendingInvites: Record<string, DmInviteRequest>;
  /** Accepted group DMs, keyed by SMPL record key. */
  conversations: Record<string, DmConversation>;
  /** Per-conversation message log, keyed by record key. */
  messages: Record<string, DmMessage[]>;
}

const [dmState, setDmState] = createStore<DmState>({
  pendingInvites: {},
  conversations: {},
  messages: {},
});

export { dmState, setDmState };
