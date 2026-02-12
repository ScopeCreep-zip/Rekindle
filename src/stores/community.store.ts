import { createStore } from "solid-js/store";
import type { Message } from "./chat.store";

export interface Channel {
  id: string;
  name: string;
  type: "text" | "voice";
  unreadCount: number;
}

export interface Member {
  publicKey: string;
  displayName: string;
  role: string;
  status: string;
}

export interface Role {
  id: string;
  name: string;
  color: string;
  permissions: string[];
}

export interface Community {
  id: string;
  name: string;
  channels: Channel[];
  members: Member[];
  roles: Role[];
}

export interface CommunityState {
  communities: Record<string, Community>;
  activeCommunity: string | null;
  activeChannel: string | null;
  channelMessages: Record<string, Message[]>;
}

const [communityState, setCommunityState] = createStore<CommunityState>({
  communities: {},
  activeCommunity: null,
  activeChannel: null,
  channelMessages: {},
});

export { communityState, setCommunityState };
