import { createStore } from "solid-js/store";
import type { DmConversation } from "../ipc/commands";

export interface DmInviteRequest {
  recordKey: string;
  from: string;
  initiatorPseudonym: string;
  isGroup: boolean;
  receivedAt: number;
}

export interface DmMessage {
  id: number;
  senderId: string;
  body: string;
  timestamp: number;
  isOwn: boolean;
}

export interface DmState {
  /** Invite-pending conversations awaiting accept/decline. */
  pendingInvites: Record<string, DmInviteRequest>;
  /** Accepted DM conversations, keyed by SMPL record key. */
  conversations: Record<string, DmConversation>;
  /** Per-conversation message log, keyed by record_key. */
  messages: Record<string, DmMessage[]>;
  /** Currently focused DM record_key (drives the chat pane). */
  activeRecordKey: string | null;
}

const [dmState, setDmState] = createStore<DmState>({
  pendingInvites: {},
  conversations: {},
  messages: {},
  activeRecordKey: null,
});

export { dmState, setDmState };
