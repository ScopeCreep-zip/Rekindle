// Sync / pairing / video / analytics / search / DM DTOs for the Tauri
// command layer. Re-exported by ../commands.ts. Used by commands/sync.ts.

export type VideoTrackLabel = "camera" | "screen" | (string & {});

export type VideoTopologyReason =
  | "initial"
  | "relay_left"
  | "relay_overloaded"
  | "explicit_request"
  | (string & {});

export interface SendVideoFrameRequest {
  streamIdHex: string;
  frameSeq: number;
  keyframe: boolean;
  timestamp: number;
  encodedPayloadB64: string;
}

export interface BackgroundSyncReport {
  communitiesChecked: number;
  recordsInspected: number;
  failedRecords: number;
  elapsedMs: number;
}

export interface LinkPreview {
  messageId: string;
  url: string;
  title?: string;
  description?: string;
  imageUrl?: string;
  siteName?: string;
  fetchedAt: number;
}

export interface PairingSession {
  pairingCode: string;
  pairingSaltHex: string;
  personalRecordKey: string;
  expiresAt: number;
  /**
   * Architecture §28.4 — the existing device's private-route blob,
   * hex-encoded. Encoded into the QR code so the new device can
   * `acceptPairingCode` without out-of-band route delivery. Empty
   * string when no route is available yet.
   */
  existingDeviceRouteBlobHex: string;
}

/**
 * Architecture §28.4 / Phase 7 W24 line 4122 — payload returned from
 * the Rust-side QR generator. The frontend renders {@link svg} via
 * `<div innerHTML={...} />`; {@link uri} is the same string encoded
 * inside the QR (offer as a "copy link" affordance for users without a
 * working camera). {@link session} echoes the underlying pairing-code
 * fields so the existing-device UI can show TTL countdown without
 * re-parsing.
 */
export interface PairingQrPayload {
  svg: string;
  uri: string;
  session: PairingSession;
}

export interface PairingAccept {
  personalRecordKey: string;
  assignedDeviceId: string;
}

export interface SyncCommunityRef {
  communityId: string;
  joinedAt: number;
  displayName: string;
}

export interface SyncManifest {
  communities: SyncCommunityRef[];
  lamport: number;
}

export interface SyncReadStateEntry {
  communityId: string;
  channelId: string;
  lastReadLamport: number;
}

export interface SyncReadState {
  entries: SyncReadStateEntry[];
}

export interface SyncPreferences {
  notificationDefaultLevel?: number;
  theme?: string;
  language?: string;
  quietHoursStart?: string;
  quietHoursEnd?: string;
  lamport: number;
}

export interface DeviceListEntry {
  deviceId: string;
  devicePublicKey: string;
  displayName: string;
  pairedAt: number;
  unpairedAt?: number;
}

export interface DeviceList {
  devices: DeviceListEntry[];
  lamport: number;
}

export interface DailySample {
  dayUnixMs: number;
  value: number;
}

export interface DailyTimeseries {
  samples: DailySample[];
}

export interface MemberMetrics {
  totalMembers: number;
  active7d: number;
  active30d: number;
  joins7d: number;
  leaves7d: number;
  retention7Of30: number;
  activePerDay: DailyTimeseries;
  joinsPerDay: DailyTimeseries;
  leavesPerDay: DailyTimeseries;
}

export interface ChannelMetrics {
  channelId: string;
  messages7d: number;
  uniquePosters7d: number;
  peakConcurrentVoice: number;
  messagesPerDay: DailyTimeseries;
  uniquePostersPerDay: DailyTimeseries;
}

export interface GrowthSample {
  dayUnixMs: number;
  memberCount: number;
}

export interface GrowthMetrics {
  samples: GrowthSample[];
}

export interface ActivityByHour {
  /** 24-element array indexed by UTC hour 0..=23. */
  hourCounts: number[];
}

export interface CommunityAnalytics {
  communityId: string;
  members: MemberMetrics;
  channels: ChannelMetrics[];
  growth: GrowthMetrics;
  activityByHour: ActivityByHour;
  storageUsage: StorageUsage;
  computedInMs: number;
}

export interface StorageUsage {
  totalBytes: number;
  messageBytes: number;
  threadMessageBytes: number;
  channelPinBytes: number;
  readStateBytes: number;
  voiceEventBytes: number;
  memberLeaveBytes: number;
  metadataBytes: number;
}

export type HasFilter =
  | "link"
  | "file"
  | "image"
  | "video"
  | "embed"
  | "poll"
  | "voice_message";

export type SearchSort = "relevance" | "newest" | "oldest";

export interface SearchFilters {
  from?: string;
  /**
   * Architecture §32 Phase 7 W23 line 4111 — community-scoped search.
   * Undefined = global (all communities the local member has joined);
   * a community id restricts matches to that community via the
   * `channels.community_id` JOIN in the FTS5 query.
   */
  inCommunity?: string;
  inChannel?: string;
  inThread?: string;
  has?: HasFilter[];
  before?: number;
  after?: number;
  mentions?: string;
  isPinned?: boolean;
}

export interface MessageSearch {
  query: string;
  filters?: SearchFilters;
  sort?: SearchSort;
  limit?: number;
  offset?: number;
}

export type SearchScope = "channel" | "thread" | "dm";

export interface SearchHit {
  scope: SearchScope;
  conversationId: string;
  messageId?: string;
  senderKey: string;
  body: string;
  timestamp: number;
  rank: number;
  beforeBody?: string;
  afterBody?: string;
}

export interface SearchResult {
  hits: SearchHit[];
  totalReturned: number;
  queryMs: number;
}

export interface DmConversation {
  recordKey: string;
  isGroup: boolean;
  initiatorPublicKey: string;
  initiatorPseudonym: string;
  mySubkey: number;
  participants: { pseudonym: string; subkey: number; publicKey: string }[];
  mekGeneration: number;
  createdAt: number;
  lastMessageAt: number | null;
}

export interface DmMessageRecord {
  id: number;
  senderPseudonym: string;
  body: string;
  timestamp: number;
  sequence: number;
  mekGeneration: number;
}

/**
 * Phase 11 Tier 1 — payload pushed through the per-peer DM video
 * `Channel` (replaces the `dm-video-frame` event). Mirrors the Rust
 * `video_channels::DmVideoFrameMsg`.
 */
export interface DmVideoFrameMsg {
  peerPubkey: string;
  streamIdHex: string;
  frameSeq: number;
  keyframe: boolean;
  timestamp: number;
  encodedPayloadB64: string;
}

/**
 * Phase 11 Tier 1 — payload pushed through the per-community video
 * `Channel` (replaces the `community-event` `videoFrame` variant).
 * Mirrors the Rust `video_channels::CommunityVideoFrameMsg`.
 */
export interface CommunityVideoFrameMsg {
  communityId: string;
  senderPseudonym: string;
  streamId: string;
  frameSeq: number;
  keyframe: boolean;
  timestamp: number;
  payloadB64: string;
}
