import { createStore } from "solid-js/store";
import type { Message } from "./chat.store";
import type { GameInfo, InviteDto, OnboardingConfig, WelcomeScreen } from "./types";

export interface Channel {
  id: string;
  name: string;
  type: "text" | "voice" | "announcement" | "forum" | "stage" | "directory" | "media" | "events" | "dm";
  unreadCount: number;
  categoryId?: string;
  topic?: string;
  slowmodeSeconds?: number;
  nsfw?: boolean;
  messageRecordKey?: string;
  mekGeneration?: number;
}

export interface Category {
  id: string;
  name: string;
  sortOrder: number;
}

export interface Member {
  pseudonymKey: string;
  displayName: string;
  roleIds: number[];
  displayRole: string;
  status: string;
  timeoutUntil: number | null;
  gameInfo: GameInfo | null;
}

export interface Role {
  id: number;
  name: string;
  color: number;
  /** Serialized as a string from Rust to avoid JavaScript Number precision loss on u64. */
  permissions: string;
  position: number;
  hoist: boolean;
  mentionable: boolean;
}

export interface EventRsvp {
  pseudonymKey: string;
  status: "going" | "maybe" | "declined";
}

export interface Thread {
  id: string;
  channelId: string;
  name: string;
  starterMessageId: string;
  creatorPseudonym: string;
  createdAt: number;
  archived: boolean;
  autoArchiveSeconds: number;
  lastMessageAt: number;
  messageCount: number;
}

export interface CommunityEvent {
  id: string;
  title: string;
  description: string;
  creatorPseudonym: string;
  startTime: number;
  endTime: number | null;
  channelId: string | null;
  maxAttendees: number | null;
  createdAt: number;
  status: "scheduled" | "active" | "completed" | "canceled";
  rsvps: EventRsvp[];
}

export interface GameServer {
  id: string;
  gameId: string;
  label: string;
  address: string;
  addedBy: string;
  createdAt: number;
}

export interface Community {
  id: string;
  name: string;
  description: string | null;
  channels: Channel[];
  categories: Category[];
  members: Member[];
  roles: Role[];
  myRoleIds: number[];
  myPseudonymKey: string | null;
  mekGeneration: number;
  events: CommunityEvent[];
  manifestKey?: string;
  memberRegistryKey?: string;
  coordinatorPseudonym?: string;
  coordinatorEpoch?: number;
  onboardingConfig?: OnboardingConfig;
  welcomeScreen?: WelcomeScreen;
  onboardingComplete?: boolean;
}

export interface VoiceChannelState {
  participants: string[];
  mode: "mesh" | "mcu";
  hostPseudonym: string | null;
}

export interface CommunityState {
  communities: Record<string, Community>;
  activeCommunity: string | null;
  activeChannel: string | null;
  channelMessages: Record<string, Message[]>;
  channelThreads: Record<string, Thread[]>;
  threadMessages: Record<string, Message[]>;
  activeThread: string | null;
  gameServers: Record<string, GameServer[]>;
  notificationOverrides: Record<string, "all" | "mentions" | "none">;
  communityInvites: Record<string, InviteDto[]>;
  voiceChannels: Record<string, VoiceChannelState>;
}

const [communityState, setCommunityState] = createStore<CommunityState>({
  communities: {},
  activeCommunity: null,
  activeChannel: null,
  channelMessages: {},
  channelThreads: {},
  threadMessages: {},
  activeThread: null,
  gameServers: {},
  notificationOverrides: {},
  communityInvites: {},
  voiceChannels: {},
});

export { communityState, setCommunityState };
