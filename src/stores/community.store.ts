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
  forumTags?: string[];
  stageSpeakers?: string[];
  stageModerator?: string | null;
  slowmodeSeconds?: number;
  nsfw?: boolean;
  messageRecordKey?: string;
  mekGeneration?: number;
  notificationLevel?: "all" | "mentions" | "nothing";
  /**
   * Architecture §32 Phase 7 Week 25 — channel-level notification sound
   * override. The value is the soundboard expression's `contentHash`
   * (BLAKE3). `null` means "inherit from the community default", which
   * itself falls back to the app-global `notification_sound` toggle.
   */
  notificationSoundRef?: string | null;
  /**
   * Architecture §10.8 — text-in-voice. When set, this channel is the
   * text companion of the named voice channel; UI hides it from the
   * channel list unless the local member is currently connected to
   * that voice channel.
   */
  parentVoiceChannelId?: string | null;
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
  /** Per-community profile bio (≤190 chars). Reader-aggregated from peer presence subkey. */
  bio?: string | null;
  pronouns?: string | null;
  themeColor?: number | null;
  badges?: string[];
  /** BLAKE3 reference for the member's avatar in this community (architecture §24.2). */
  avatarRef?: string | null;
  /** BLAKE3 reference for the member's banner in this community. */
  bannerRef?: string | null;
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
  selfAssignable?: boolean;
  /**
   * Architecture §19.4 — when set, the member can hold at most one
   * role per group. The CRDT auto-unassigns peers in the same group
   * with a lower Lamport.
   */
  exclusionGroup?: string;
}

export interface SoundboardMeta {
  /** Architecture §18.3 — duration of the clip, ≤5 seconds. */
  durationSeconds: number;
  /** 0.0–1.0 multiplier the receivers apply to channel volume. */
  volume: number;
  /** Optional Unicode glyph the picker shows next to the name. */
  emoji?: string;
}

export interface Expression {
  id: string;
  name: string;
  kind: "emoji" | "sticker" | "soundboard";
  contentHash: string;
  inlineDataBase64?: string | null;
  inlineDataUrl?: string | null;
  mediaType?: string | null;
  animated: boolean;
  tags: string[];
  /** Architecture §18.3 — present only when kind === "soundboard". */
  soundMeta?: SoundboardMeta;
  /** Architecture §18.1 — uploader's per-community pseudonym (hex). */
  creatorPseudonym?: string;
  /** Architecture §18.1 — wall-clock seconds at upload. */
  createdAt?: number;
  /** Architecture §18.1 — gates `USE_EXTERNAL_EMOJIS` cross-community use. */
  availableToPeers?: boolean;
}

export interface AutoModRule {
  ruleId: string;
  name: string;
  enabled: boolean;
  keywords: string[];
  regexPatterns: string[];
  action: "block_locally" | "blur_content" | "alert_moderators";
  lamport: number;
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
  forumTag?: string | null;
  createdAt: number;
  archived: boolean;
  autoArchiveSeconds: number;
  lastMessageAt: number;
  messageCount: number;
}

export type RecurrenceFrequency = "daily" | "weekly" | "monthly";

export type DayOfWeek =
  | "sunday"
  | "monday"
  | "tuesday"
  | "wednesday"
  | "thursday"
  | "friday"
  | "saturday";

export interface RecurrenceRule {
  frequency: RecurrenceFrequency;
  interval: number;
  daysOfWeek?: DayOfWeek[];
  until?: number;
  count?: number;
}

export type EventLocation =
  | { type: "voice_channel"; data: string }
  | { type: "stage_channel"; data: string }
  | { type: "external"; data: string }
  | { type: "in_game"; data: { gameId: number; serverAddress?: string } };

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
  status: "scheduled" | "active" | "completed" | "cancelled";
  rsvps: EventRsvp[];
  coverImageRef?: string;
  recurrence?: RecurrenceRule;
  location?: EventLocation;
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
  /** Architecture §32 Phase 5 W15 — community-level icon BLAKE3 hex hash.
   *  Resolved to a `data:image/webp;base64,…` URL via
   *  `getCommunityAvatarDataUrl(communityId, hash)` and cached in
   *  `iconDataUrl` so the buddy-list icon doesn't re-fetch on every render. */
  iconHash?: string | null;
  /** Cached `data:image/webp;base64,…` URL for `iconHash`. `null` until the
   *  resolver runs after hydration; updated when `iconHash` changes. */
  iconDataUrl?: string | null;
  /** Community-level banner BLAKE3 hex hash. */
  bannerHash?: string | null;
  /** Cached data URL for `bannerHash`. */
  bannerDataUrl?: string | null;
  channels: Channel[];
  categories: Category[];
  members: Member[];
  roles: Role[];
  myRoleIds: number[];
  myPseudonymKey: string | null;
  mekGeneration: number;
  events: CommunityEvent[];
  memberRegistryKey?: string;
  governanceKey: string | null;
  onboardingConfig?: OnboardingConfig;
  welcomeScreen?: WelcomeScreen;
  onboardingComplete?: boolean;
  expressions: Expression[];
  automodRules: AutoModRule[];
  /** Our per-community profile bio (≤190 chars). Local-only; resets to undefined on restart. */
  myBio?: string | null;
  myPronouns?: string | null;
  myThemeColor?: number | null;
  myBadges?: string[];
  /** Per-community avatar BLAKE3 content reference (architecture §24.2). */
  myAvatarRef?: string | null;
  /** Per-community banner BLAKE3 content reference (architecture §24.2). */
  myBannerRef?: string | null;
  /** Lost Cargo: attachment_id hex strings that admins have pinned (exempt
   *  from local LRU eviction). Mirrored from the merged governance state. */
  pinnedAttachments?: string[];
  /** Plate Gate (architecture §15): merged segment metadata. Each entry
   *  describes one expansion segment that admins added when the prior
   *  segment hit its 255-slot cap. Segment 0 (genesis) is implicit and
   *  not present in this list. */
  segments?: SegmentInfo[];
  /** Which segment hosts our slot. 0 for the primary; 1..=MAX_SEGMENTS
   *  for expansion segments. */
  mySegmentIndex?: number;
  /** Architecture §17.4 — raid alert flag toggled by the
   *  `raidAlert` community-event. The CommunityWindow renders a
   *  banner overlay while this is `true`; cleared by the matching
   *  `raidAlert { active: false }` event from the backend. */
  raidAlertActive?: boolean;
}

export interface SegmentInfo {
  segmentIndex: number;
  registryKey: string;
  governanceKey: string;
  slotRangeStart: number;
  slotRangeEnd: number;
}

export interface VoiceChannelState {
  participants: string[];
  mode: "mesh" | "mcu";
  hostPseudonym: string | null;
  speakers?: string[];
  moderatorPseudonym?: string | null;
  topic?: string | null;
  pendingRequests?: string[];
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
  communityInvites: {},
  voiceChannels: {},
});

export { communityState, setCommunityState };
