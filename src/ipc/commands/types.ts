// Shared IPC DTOs for the Tauri command layer. Extracted from the
// former monolithic commands.ts; re-exported by ../commands.ts so
// existing `import { ... } from "../ipc/commands"` call sites work.

import type { OwnRecordsPhase, OwnRouteClass, RouteAvailability, RoutePauseCause } from "../channels/notification_events";


/** D22's delivery vocabulary (`rekindle_types::message::SendState`). */
/** Why a stored message's body could not be read (the Host's words in `reasonText`). */
type UndecryptableReason = "damaged";

export interface Undecryptable {
  reason: UndecryptableReason;
  reasonText: string;
}

export type SendState = "awaiting_key" | "queued" | "sent" | "delivered" | "read" | "failed";

export interface Message {
  id: number;
  senderId: string;
  body: string;
  decryptionFailed?: boolean;
  /** Stored without its body, and why (KP: the Host's reason and its words,
   *  shown in place of the body); absent or null for a readable message. */
  undecryptable?: Undecryptable | null;
  automodBlurred?: boolean;
  timestamp: number;
  isOwn: boolean;
  serverMessageId?: string;
  reactions?: { emoji: string; count: number; reactors: string[] }[];
  pinned?: boolean;
  poll?: {
    pollId: string;
    question: string;
    answers: { index: number; text: string; voteCount: number; voters: string[] }[];
    multiSelect: boolean;
    expiresAt?: number;
    closed: boolean;
    selectedAnswers: number[];
  };
  forwardedFromAuthor?: string | null;
  /** The forward's author as the community names them (the display form a
   *  sender header shows); absent when not a forward. */
  forwardedFromName?: string | null;
  sendState: SendState;
  /** Why our `failed` post was not delivered, as the backend words it
   *  (plan E3.2 S9, F9-3; S7l's key deadline); absent otherwise. */
  failureReason?: string;
  /** When its author last edited it, in seconds (plan E3.3); absent when
   *  unedited. Stored on every reader, so it survives a reload. */
  editedAt?: number;
  /** The moderator's pseudonym when a moderator removed it (plan E3.3 M4):
   *  its body is gone and the window shows "Removed by". */
  removedBy?: string;
  /** A history-gap row, not a message (plan E3.3, D23): the sender's
   *  channel outbox reaches this chain seq and no read reached it. */
  historyGapThrough?: number | null;
  attachment?: {
    attachmentId: string;
    filename: string;
    mimeType: string;
    totalSize: number;
    chunkCount: number;
    downloaded: boolean;
  };
  flags?: number;
}

export interface SoundboardMeta {
  /** Architecture §18.3 — duration of the clip, ≤5 seconds. */
  durationSeconds: number;
  /** 0.0–1.0 multiplier the receivers apply to channel volume. */
  volume: number;
  /** Optional Unicode glyph the picker shows next to the name. */
  emoji?: string;
}

export interface ExpressionInfo {
  expressionId: string;
  name: string;
  kind: "emoji" | "sticker" | "soundboard";
  contentHash: string;
  /** `data:` URL built by the backend from an allowlisted media type. */
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
  availableToPeers: boolean;
}

export interface QuietHoursSettings {
  enabled: boolean;
  startHour: number;
  endHour: number;
  /**
   * Architecture §17.2 — IANA timezone identifier (e.g.,
   * `"America/Los_Angeles"`). The stored row defaults to `"UTC"`;
   * backend uses `chrono-tz` so DST is automatic.
   */
  timezone: string;
}

export interface AutoModRuleInfo {
  ruleId: string;
  name: string;
  enabled: boolean;
  keywords: string[];
  regexPatterns: string[];
  action: "blockLocally" | "blurContent" | "alertModerators";
  lamport: number;
}

export interface SendChannelMessageResult {
  status: SendState;
  messageId: string;
}

export interface FriendInfo {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: string;
  statusMessage: string | null;
  gameInfo: GameStatus | null;
  /** Our group they are in, the id the group commands take; null for none. */
  groupId: number | null;
  group: string | null;
  /** The group the buddy list shows them under: `group`, or the Host's
   *  default group name when `group` is null. */
  groupLabel: string;
  lastSeenAt: number | null;
  friendshipState: "pendingOut" | "accepted";
  /** Why the secure session could not be set up, while it is failed (C7.25). */
  handshakeError: string | null;
  /** Our first contact is being resent from a fresh key (plan E2.4, B6). */
  handshakeRetrying: boolean;
  /** Their keys are not signed by the identity we know for them: a trust
   *  alert shown on their row until it is resolved (plan E2.4, B3). */
  identityPinMismatch: boolean;
  /** Why the Host cannot put them in a new group DM yet; `null` when it
   *  can (plan E2.6, GC3). */
  groupDmBlock: string | null;
}

interface GameStatus {
  gameId: number;
  gameName: string;
  serverInfo: string | null;
  elapsedSeconds: number;
  serverAddress: string | null;
  /** `src/assets/game-icons/<key>.png`, resolved by the backend (plan E7.2). */
  iconKey: string | null;
  /** Whether "Join Game" can start this game on their server, stated by the
   *  Host's launch check (plan E7.15 IV3c). */
  joinable: boolean;
  /** Why not, while `joinable` is false. */
  joinableReason: NotJoinableReason | null;
}

/** `rekindle_types::friend::NotJoinableReason`. */
export type NotJoinableReason = "noServer" | "cannotLaunch";

export interface AudioDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
  /** Input channels an input-channel choice picks from; 0 for outputs. */
  channels: number;
}

export interface AudioDevices {
  inputDevices: AudioDeviceInfo[];
  outputDevices: AudioDeviceInfo[];
}

/** Plan §Failure 5 — row from `getMissedCalls`. `kind` is 0 = audio, 1 = video. */
export interface MissedCallRow {
  callId: string;
  peerKey: string;
  kind: number;
  expiredAt: number;
}

/** `Preferences.notificationContent`: sender and message, sender only, or nothing. */
export type NotificationContent = "nameAndMessage" | "nameOnly" | "nothing";

/// What arming or disarming the quick wipe answers (`QuickWipeState`).
export interface QuickWipeState {
  armed: boolean;
  shortcut: string | null;
}

export interface Preferences {
  notificationsEnabled: boolean;
  notificationSound: boolean;
  startMinimized: boolean;
  autoStart: boolean;
  gameDetectionEnabled: boolean;
  gameScanIntervalSecs: number;
  inputDevice: string | null;
  outputDevice: string | null;
  /** Input-channel choice per input device name, 0-based. A device without
   *  an entry averages all its channels. */
  inputChannels: Record<string, number[]>;
  videoDeviceId: string | null;
  videoDeviceLabel: string | null;
  inputVolume: number;
  outputVolume: number;
  noiseSuppression: boolean;
  echoCancellation: boolean;
  autoAwayMinutes: number;
  /** W11.3 — auto-volunteer Strand Relay for new friend accepts. */
  autoVolunteerRelayForNewFriends: boolean;
  /** Wave 12 W12.2 — ringtone gates + DND auto-suppress. */
  ringtoneEnabled?: boolean;
  ringtoneVolume?: number;
  inCallDndAutoEnable?: boolean;
  /** What an OS notification shows; the Host decides and enforces it. */
  notificationContent?: NotificationContent;
  /** DM read receipts (plan E2.5): reciprocal, off by default. Changed only
   *  by `setReadReceiptsEnabled`, never by `setPreferences`. */
  readReceiptsEnabled: boolean;
  /** Send our game totals to friends (plan E7.2): opt-in, off by default;
   *  set only by `setShareGameTotals`, never by `setPreferences`. */
  shareGameTotals: boolean;
  /** Show friends the game we play (plan E7.2): split from detection, off
   *  by default; set only by `setShareCurrentGame`. */
  shareCurrentGame: boolean;
  /** Keep the local activity feed (plan E7.15): on by default, local
   *  only; set only by `SetKeepActivityLog`, which clears it when off. */
  keepActivityLog: boolean;
}

/** What a feed entry says (`rekindle_types::activity::ActivityKind`). */
export type ActivityKind =
  | { type: "startedGame"; data: { friend: string; gameId: number; server: string | null } }
  | { type: "stoppedGame"; data: { friend: string; gameId: number; seconds: number } }
  | { type: "firstPlayed"; data: { friend: string; gameId: number } }
  | { type: "renamedSelf"; data: { friend: string; from: string; to: string } }
  | { type: "statusMessage"; data: { friend: string; text: string } }
  | { type: "bioChanged"; data: { friend: string } }
  | { type: "nowFriends"; data: { friend: string } }
  | { type: "favouriteAdded"; data: { gameId: number; address: string; label: string } }
  | { type: "joinedCommunity"; data: { community: string } };

/** A feed entry as `ActivityList` answers and `ActivityAppended` pushes it
 *  (plan E7.15): names are the Host's at read time. */
export interface ActivityListItem {
  id: number;
  at: number;
  kind: ActivityKind;
  displayName: string | null;
  nickname: string | null;
  identityPinMismatch: boolean;
}

/** A game on the hide list (plan E7.2): never shown to anyone. */
export interface HiddenGameView {
  gameId: number;
  gameName: string;
  iconKey: string | null;
}

/** Where a launch's server came from (`JoinSource`, lead decision (50)):
 *  the Host reviews a peer's server first, naming it. */
export type JoinSource = "installed" | "favourite" | "recent" | "friendPresence" | "invite" | "community" | "lobby";

/** A game we can launch (`InstalledGamesGet`, plan E7.15 IV3). */
export interface InstalledGameView {
  gameId: number;
  gameName: string;
  iconKey: string | null;
  /** Whether a launch without a server can start it. */
  launchable: boolean;
}

/** A personal favourite game server (plan E7.2), as the Host stores it.
 *  `invitable` is the Host's answer to whether an invite may name it: the
 *  game is not hidden and current-game sharing is on. */
export interface FavouriteServerView {
  gameId: number;
  gameName: string;
  iconKey: string | null;
  address: string;
  label: string;
  addedAt: number;
  invitable: boolean;
}

export type ExclusionGroupEdit =
  | { set: string }
  | "clear"
  | "unchanged";

export interface CommunityPolicy {
  /** Optional Markdown rules text (architecture §10.7 line 724). */
  policyText?: string;
  /** Architecture §20.6 — joins-per-interval threshold (default 20). */
  maxJoinsPerInterval: number;
  /** Architecture §20.6 — interval length in seconds (default 600). */
  joinIntervalSeconds: number;
}

/** How coarse a peer's last-seen timestamp is exposed. */
type LastSeenPrecision = "hidden" | "coarse" | "exact";

/** Per-signal share scope. Default-deny: "none" hides the signal. */
type ShareScope = "none" | "members";

/**
 * Per-community presence sharing consent (default-deny). Local-only — never
 * published to the registry. Gates what our own presence row reveals and,
 * by reciprocity, which peer signals we get to read.
 */
export interface PresenceSharingPolicy {
  /** Master switch: appear online at all. Off ⇒ Invisible to peers. */
  shareOnline: boolean;
  /** Share which channel we're in (text/voice). */
  shareLocation: ShareScope;
  /** Share activity / game string. */
  shareActivity: ShareScope;
  /** Last-seen granularity exposed to peers. */
  lastSeen: LastSeenPrecision;
}

export interface NetworkStatus {
  attachmentState: string;
  isAttached: boolean;
  publicInternetReady: boolean;
  /** Our inbound (General) route's availability (plan F9-A). */
  generalRoute: RouteAvailability;
  mediaRoute: RouteAvailability;
  /** The Host accepts writes now (its lifecycle's `can_write`). */
  canWrite: boolean;
  /** The indicator's bars as shown now, 0–5 (plan E7.2). */
  bars: number;
  /** Where the own-records stage is (plan E2.5 FC3). */
  ownRecords: OwnRecordsPhase;
  /** Why the stage failed, when it did. */
  ownRecordsError: string | null;
  /** Our inbound route is paused and why (plan RV1c), or null when it is not. */
  routePaused: RoutePauseCause | null;
  /** Our own routes down while the Host retries (plan F9-A): since when (epoch ms) and why, per class, or null. */
  ownRouteDown: Record<OwnRouteClass, { sinceMs: number; reason: string } | null>;
  profileDhtKey: string | null;
  friendListDhtKey: string | null;
}

// Architecture §21 — scheduled-event metadata.
type RecurrenceFrequency = "daily" | "weekly" | "monthly";

type DayOfWeek =
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

export interface CreateEventRequest {
  title: string;
  description: string;
  startTime: number;
  endTime?: number;
  channelId?: string;
  maxAttendees?: number;
  /** Architecture §21 line 2624 — peer-cached cover image hex hash. */
  coverImageRef?: string;
  /** Architecture §21 line 2628 — recurrence rule (omit for one-off). */
  recurrence?: RecurrenceRule;
  /** Architecture §21 line 2629 — event location. */
  location?: EventLocation;
}

interface EventRsvpInfo {
  pseudonymKey: string;
  status: string;
}

export interface EventInfo {
  id: string;
  title: string;
  description: string;
  creatorPseudonym: string;
  startTime: number;
  endTime: number | null;
  channelId: string | null;
  maxAttendees: number | null;
  createdAt: number;
  status: string;
  rsvps: EventRsvpInfo[];
  coverImageRef?: string;
  recurrence?: RecurrenceRule;
  location?: EventLocation;
}

// Declared once in the IPC layer — community.store.ts re-exports both
// (`export type { Thread, GameServer }`) rather than defining them, so
// every existing consumer that imports from the store keeps working.
// Moved here because `ipc/channels/community_subscription_events.ts`
// needs them too, and src/ipc/ is the dependency-cruiser-enforced leaf:
// it may not import from src/stores/.

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

export interface GameServer {
  id: string;
  gameId: string;
  label: string;
  address: string;
  addedBy: string;
  createdAt: number;
}

/// A settings-window tab. Mirrors `SettingsTab` in `src-tauri/src/windows/mod.rs`.
export type SettingsTab =
  | "profile"
  | "application"
  | "appearance"
  | "notifications"
  | "audio"
  | "video"
  | "privacy"
  | "devices"
  | "mobile"
  | "about";

/// A live 1:1 call (`get_active_call`).
export interface ActiveCall {
  callId: string;
  peerKey: string;
  displayName: string;
  kind: "audio" | "video";
  expiresAtMs: number;
  /// Accepted (connecting or active) rather than still ringing out.
  connected: boolean;
}

/** Our own presence status as the Host states it (`SessionStatus`). */
export type SessionStatus = "online" | "away" | "busy" | "offline" | "invisible";
