import { batch } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import type {
  OpPublicationState,
  OpQuarantineReason,
} from "../ipc/channels/community_governance_events";
import type {
  GameInfo,
  InviteDto,
  MemberGroup,
  OnboardingConfig,
  WelcomeScreen,
} from "../ipc/commands/dto";
import type {
  ChannelDeniedCapabilities,
  DeniedCapabilities,
  MemberModeration,
} from "../ipc/commands/dto_capabilities";
import type { Message } from "./chat.store";

export interface Channel {
  id: string;
  name: string;
  /** The channel controls we lack here, each with the Host's reason
   *  (`CommunityChannelView::denied`, judged with the channel's overwrites). */
  denied?: ChannelDeniedCapabilities;
  type:
    | "text"
    | "voice"
    | "announcement"
    | "forum"
    | "stage"
    | "directory"
    | "media"
    | "events"
    | "dm";
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
  /** A restricted channel whose roster we are not on (FX3): listed, closed. */
  locked?: boolean;
  /** The Host's words for why it is locked. */
  lockedReason?: string;
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
  /** The Host's name, never empty: the per-community card name, else the short fingerprint (plan E3.4 S6). */
  displayName: string;
  /** The member's short pseudonym fingerprint ("a1b2 c3d4"), as the Host gives it. */
  fingerprint: string;
  /** The card's custom status, or null. */
  customStatus: string | null;
  roleIds: string[];
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
  /** Where the member is focused right now (if they share it). Drives the
   * "in #channel" roster grouping. `kind` is "text" | "voice". */
  location?: MemberLocation | null;
  /** Member's last-active unix seconds, already coarsened by their sharing
   * policy (0 / undefined = not shared). Drives the "last seen" label. */
  lastActive?: number;
  /** The Host's member-list group; the list renders it as given. */
  group: MemberGroup;
  /** Which moderation actions we may take against this member: the
   *  governance validator's verdicts, computed by the Host (plan E3.2 S9). */
  moderation: MemberModeration;
  /** Whether we outrank this member (the validator's rank rule). */
  outranks: boolean;
}

/** A member's focused channel, decoded from their shared session. */
export interface MemberLocation {
  kind: "text" | "voice";
  channelId: string;
}

/** `@everyone`: the all-zero 16-byte role id every member holds. */
export const EVERYONE_ROLE_ID = "0".repeat(32);

/** A role tag's text colour per skin base, `0xRRGGBB` (the Host's `RoleTagColours`). */
export interface RoleTagColours {
  dark: number;
  light: number;
}

export interface Role {
  /** The full 16-byte role id, 32 hex characters (plan E3.2 S9). */
  id: string;
  name: string;
  color: number;
  tagColours: RoleTagColours;
  /** Serialized as a string from Rust to avoid JavaScript Number precision loss on u64. */
  permissions: string;
  position: number;
  /** The position as the Host presents it (contract PN1: the owner role reads
   *  "Always first", never the raw sentinel). */
  positionLabel: string;
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

import type { DeviceStatus } from "../ipc/commands/community";
// Declared once in the IPC layer, which mirrors the Rust contract.
import type { SoundboardMeta } from "../ipc/commands/types";

export interface Expression {
  id: string;
  name: string;
  kind: "emoji" | "sticker" | "soundboard";
  contentHash: string;
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
  action: "blockLocally" | "blurContent" | "alertModerators";
  lamport: number;
}

export interface EventRsvp {
  pseudonymKey: string;
  status: "going" | "maybe" | "declined";
}

// Scheduled-event types (architecture §21) — declared once in the IPC
// layer; these were byte-identical copies. Thread and GameServer moved
// the same way once `ipc/channels/community_subscription_events.ts`
// needed them too and src/ipc/ (the dependency-cruiser leaf) couldn't
// import them back out of this store.
import type { EventLocation, GameServer, RecurrenceRule, Thread } from "../ipc/commands/types";

export type { GameServer, Thread };

import type { CatchUpView, KeyStatus } from "../ipc/channels/community_governance_events";

/**
 * A scheduled community event (architecture §21) — the calendar entry
 * shown in the events panel.
 *
 * Named `CommunityEvent` until it collided with
 * `ipc/channels/community_events.ts`'s `CommunityEvent`, which is the
 * Rust `CommunityEvent` enum arriving over IPC. Two exported types, one
 * name, entirely different meanings.
 */
export interface ScheduledEvent {
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
  /** Whether `members` holds the Host's member list: `loading` until its
   *  first `get_community_members` answer, `known` from then on, or the
   *  reason the first read failed. Until it is known the panel claims no
   *  count: an empty list before the first answer is "not known", not "no
   *  members". */
  membersState: MembersState;
  roles: Role[];
  myRoleIds: string[];
  /** Our base permission bits (decimal u64 string), from the backend's
   *  merged governance (plan E3.2 S9); the window computes no policy. */
  myPermissions: string;
  /** Our effective permissions per channel id: base plus overwrites. */
  myChannelPermissions: Record<string, string>;
  myPseudonymKey: string | null;
  /** Whether we hold the community's current key (FX3). */
  keyStatus?: KeyStatus | null;
  /** The governance writes the unlock barrier holds, with the Host's label
   *  for each (`CatchUp`; plan E3.2 S7d-keys.10): shown while pending. */
  catchUp?: CatchUpView | null;
  /** What we may not do here and why, as the Host says (never-hide): a
   *  control is disabled with the reason, never hidden. */
  denied?: DeniedCapabilities;
  /** Bumped on each `BansChanged` (plan E3.2 S9): a shown ban list re-reads on it. */
  bansRevision?: number;
  /** Set when we were removed (plan E3.2 S9 own removal): the community
   *  stays as read-only history. */
  /** Set when the inviter refused our join after it returned (plan JW):
   *  the Host's words, shown in place in this community's window. */
  withdrawn?: { reasonText: string } | null;
  removed?: {
    banned: boolean;
    by: string | null;
    byName: string | null;
    reason?: string | null;
  } | null;
  mekGeneration: number;
  events: ScheduledEvent[];
  memberRegistryKey?: string;
  anchor: string | null;
  onboardingConfig?: OnboardingConfig;
  welcomeScreen?: WelcomeScreen;
  onboardingComplete?: boolean;
  /** We hold a creator seat, as the Host states it (`CommunityView::we_are_creator`). */
  weAreCreator: boolean;
  /** Whether this device may write here, as the Host states it (`CommunityView::device_status`). */
  deviceStatus: DeviceStatus;
  /** Our name in this community, never empty, as the Host states it (plan E3.4 S6); never our identity name. */
  myDisplayName: string;
  /** Our per-community name as set, or null (the set-name form's prefill). */
  myCardName: string | null;
  /** Our card's custom status, or null. */
  myCustomStatus: string | null;
  expressions: Expression[];
  automodRules: AutoModRule[];
  /** Our per-community profile bio (≤190 chars), as the Host persists it (plan E3.4 S6b). */
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
}

interface SegmentInfo {
  segmentIndex: number;
  registryKey: string;
  anchor: string;
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

interface CommunityState {
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
  /** Our governance ops not yet landed, per community then op hash, as the
   *  backend announces them (pending, or failed with its reason). */
  opPublications: Record<string, Record<string, OpPublicationState>>;
  /** Governance ops the backend set aside, per community then op hash
   *  (`governanceOpQuarantined` events): shown, never applied. */
  quarantinedOps: Record<string, Record<string, QuarantinedOp>>;
  /** Communities whose inviter's snapshot did not match the full history
   *  (`checkpointSnapshotMismatch` events): the window keeps the notice. */
  snapshotMismatch: Record<string, true>;
}

/** One quarantined governance op, as the backend announced it. */
interface QuarantinedOp {
  author: string;
  reason: OpQuarantineReason;
  cause: string | null;
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
  opPublications: {},
  quarantinedOps: {},
  snapshotMismatch: {},
});

export { communityState, setCommunityState };

/**
 * Write a re-read of `community`'s member list from the Host (MM1). The
 * rows are reconciled by pseudonym key, so a member already listed keeps
 * its store object and its row: a re-read updates the row in place instead
 * of replacing it, and a context menu or profile open on that row stays
 * open across the re-reads a join, a rejoin or a role change sets off.
 */
export function setCommunityMembers(community: string, members: Member[]): void {
  batch(() => {
    setCommunityState(
      "communities",
      community,
      "members",
      reconcile(members, { key: "pseudonymKey" })
    );
    setCommunityState("communities", community, "membersState", "known");
  });
}

/** Whether a community's member list is the Host's yet (see `Community.membersState`). */
export type MembersState = "loading" | "known" | { failed: string };

/**
 * A member read for `community` failed with `reason`: shown only while no
 * list is known yet. A later refresh that fails keeps the list the panel
 * already shows.
 */
export function markMembersFailed(community: string, reason: string): void {
  const c = communityState.communities[community];
  if (!c || c.membersState === "known") return;
  setCommunityState("communities", community, "membersState", { failed: reason });
}

/**
 * Drop community `id` from this window's list, and the selection when it
 * was the shown one. The key is deleted in place (`produce`): a setter given
 * a copy without the key merges it into the store, so the row stayed.
 */
export function dropCommunity(id: string): void {
  setCommunityState(
    "communities",
    produce((communities) => {
      delete communities[id];
    })
  );
  if (communityState.activeCommunity === id) {
    setCommunityState("activeCommunity", null);
    setCommunityState("activeChannel", null);
  }
}
