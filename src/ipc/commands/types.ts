// Shared IPC DTOs for the Tauri command layer. Extracted from the
// former monolithic commands.ts; re-exported by ../commands.ts so
// existing `import { ... } from "../ipc/commands"` call sites work.

export interface LoginResult {
  publicKey: string;
  displayName: string;
}

export interface IdentitySummary {
  publicKey: string;
  displayName: string;
  createdAt: number;
  hasAvatar: boolean;
  avatarBase64: string | null;
}

export interface Message {
  id: number;
  senderId: string;
  body: string;
  decryptionFailed?: boolean;
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
  attachment?: {
    attachmentId: string;
    filename: string;
    mimeType: string;
    totalSize: number;
    chunkCount: number;
    localPath?: string | null;
  };
  flags?: number;
}

export interface SoundboardMeta {
  durationSeconds: number;
  volume: number;
  emoji?: string;
}

export interface ExpressionInfo {
  expressionId: string;
  name: string;
  kind: "emoji" | "sticker" | "soundboard";
  contentHash: string;
  inlineDataBase64?: string | null;
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
   * `"America/Los_Angeles"`). Frontend seeds with
   * `Intl.DateTimeFormat().resolvedOptions().timeZone` on first
   * configuration; backend uses `chrono-tz` so DST is automatic.
   */
  timezone: string;
}

export interface AutoModRuleInfo {
  ruleId: string;
  name: string;
  enabled: boolean;
  keywords: string[];
  regexPatterns: string[];
  action: "block_locally" | "blur_content" | "alert_moderators";
  lamport: number;
}

export interface SendChannelMessageResult {
  status: "delivered" | "queued";
  messageId: string;
}

export interface FriendInfo {
  publicKey: string;
  displayName: string;
  nickname: string | null;
  status: string;
  statusMessage: string | null;
  gameInfo: GameStatus | null;
  group: string | null;
  unreadCount: number;
  lastSeenAt: number | null;
  friendshipState: "pendingOut" | "accepted";
}

export interface GameStatus {
  gameId: number;
  gameName: string;
  serverInfo: string | null;
  elapsedSeconds: number;
  serverAddress: string | null;
}

export interface AudioDeviceInfo {
  id: string;
  name: string;
  isDefault: boolean;
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

export interface Preferences {
  notificationsEnabled: boolean;
  notificationSound: boolean;
  startMinimized: boolean;
  autoStart: boolean;
  gameDetectionEnabled: boolean;
  gameScanIntervalSecs: number;
  inputDevice: string | null;
  outputDevice: string | null;
  videoDeviceId: string | null;
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
}

export type ExclusionGroupEdit =
  | { kind: "set"; value: string }
  | { kind: "clear" };

export interface CommunityPolicy {
  /** Optional Markdown rules text (architecture §10.7 line 724). */
  policyText?: string;
  /** Architecture §20.6 — joins-per-interval threshold (default 20). */
  maxJoinsPerInterval: number;
  /** Architecture §20.6 — interval length in seconds (default 600). */
  joinIntervalSeconds: number;
}

/** How coarse a peer's last-seen timestamp is exposed. */
export type LastSeenPrecision = "hidden" | "coarse" | "exact";

/** Per-signal share scope. Default-deny: "none" hides the signal. */
export type ShareScope = "none" | "members";

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
  hasRoute: boolean;
  profileDhtKey: string | null;
  friendListDhtKey: string | null;
}

// Architecture §21 — scheduled-event metadata.
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

export type EventStatus = "scheduled" | "active" | "completed" | "cancelled";

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

export interface EventRsvpInfo {
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
