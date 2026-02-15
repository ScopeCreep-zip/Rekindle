import { createStore } from "solid-js/store";
import type { Message } from "./chat.store";

export interface Channel {
  id: string;
  name: string;
  type: "text" | "voice";
  unreadCount: number;
}

export interface Member {
  pseudonymKey: string;
  displayName: string;
  roleIds: number[];
  displayRole: string;
  status: string;
  timeoutUntil: number | null;
}

export interface Role {
  id: number;
  name: string;
  color: number;
  permissions: number;
  position: number;
  hoist: boolean;
  mentionable: boolean;
}

export interface Community {
  id: string;
  name: string;
  description: string | null;
  channels: Channel[];
  members: Member[];
  roles: Role[];
  myRoleIds: number[];
  myPseudonymKey: string | null;
  mekGeneration: number;
  isHosted: boolean;
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
