import { invoke } from "./invoke";
import type { OnboardingConfig, WelcomeScreen, OnboardingAnswer, GossipDiagnostics } from "../stores/types";

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

export const commands = {
  // Auth
  createIdentity: (passphrase: string, displayName?: string) =>
    invoke<LoginResult>("create_identity", { passphrase, displayName: displayName ?? null }),
  login: (publicKey: string, passphrase: string) =>
    invoke<LoginResult>("login", { publicKey, passphrase }),
  getIdentity: () => invoke<LoginResult | null>("get_identity"),
  logout: () => invoke<void>("logout"),
  listIdentities: () => invoke<IdentitySummary[]>("list_identities"),
  deleteIdentity: (publicKey: string, passphrase: string) =>
    invoke<void>("delete_identity", { publicKey, passphrase }),

  // Chat
  prepareChatSession: (peerId: string) =>
    invoke<void>("prepare_chat_session", { peerId }),
  sendMessage: (to: string, body: string) =>
    invoke<void>("send_message", { to, body }),
  sendTyping: (peerId: string, typing: boolean) =>
    invoke<void>("send_typing", { peerId, typing }),
  getMessageHistory: (peerId: string, limit: number) =>
    invoke<Message[]>("get_message_history", { peerId, limit }),
  markRead: (peerId: string) => invoke<void>("mark_read", { peerId }),

  // Friends
  addFriend: (publicKey: string, displayName: string, message: string) =>
    invoke<void>("add_friend", { publicKey, displayName, message }),
  removeFriend: (publicKey: string) =>
    invoke<void>("remove_friend", { publicKey }),
  acceptRequest: (publicKey: string, displayName: string) =>
    invoke<void>("accept_request", { publicKey, displayName }),
  rejectRequest: (publicKey: string) =>
    invoke<void>("reject_request", { publicKey }),
  getFriends: () => invoke<FriendInfo[]>("get_friends"),
  getPendingRequests: () =>
    invoke<{ publicKey: string; displayName: string; message: string; receivedAt: number }[]>(
      "get_pending_requests",
    ),
  createFriendGroup: (name: string) =>
    invoke<number>("create_friend_group", { name }),
  renameFriendGroup: (groupId: number, name: string) =>
    invoke<void>("rename_friend_group", { groupId, name }),
  moveFriendToGroup: (publicKey: string, groupId: number | null) =>
    invoke<void>("move_friend_to_group", { publicKey, groupId }),
  generateInvite: () =>
    invoke<{ url: string; inviteId: string }>("generate_invite"),
  addFriendFromInvite: (inviteString: string) =>
    invoke<void>("add_friend_from_invite", { inviteString }),
  cancelInvite: (inviteId: string) =>
    invoke<void>("cancel_invite", { inviteId }),
  getOutgoingInvites: () =>
    invoke<{ inviteId: string; url: string; createdAt: number; expiresAt: number; status: string; acceptedBy: string | null }[]>(
      "get_outgoing_invites",
    ),
  blockUser: (publicKey: string, displayName?: string) =>
    invoke<void>("block_user", { publicKey, displayName: displayName ?? null }),
  unblockUser: (publicKey: string) =>
    invoke<void>("unblock_user", { publicKey }),
  getBlockedUsers: () =>
    invoke<{ publicKey: string; displayName: string; blockedAt: number }[]>("get_blocked_users"),
  cancelRequest: (publicKey: string) =>
    invoke<void>("cancel_request", { publicKey }),
  emitFriendsPresence: () =>
    invoke<void>("emit_friends_presence"),

  // Community
  getCommunities: () =>
    invoke<{ id: string; name: string; channelCount: number }[]>("get_communities"),
  getCommunityDetails: () =>
    invoke<{
      id: string;
      name: string;
      description: string | null;
      iconHash?: string | null;
      bannerHash?: string | null;
      channels: { id: string; name: string; channelType: string; unreadCount: number; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null }[];
      categories?: { id: string; name: string; sortOrder: number }[];
      myRole: string | null;
      myRoleIds: number[];
      roles: { id: number; name: string; color: number; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean; exclusionGroup?: string }[];
      myPseudonymKey: string | null;
      mekGeneration: number;
      onboardingComplete: boolean;
      myBio?: string | null;
      myPronouns?: string | null;
      myThemeColor?: number | null;
      myBadges?: string[];
    }[]>("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    invoke<{
      pseudonymKey: string;
      displayName: string;
      roleIds: number[];
      displayRole: string;
      status: string;
      timeoutUntil: number | null;
      bio?: string | null;
      pronouns?: string | null;
      themeColor?: number | null;
      badges?: string[];
    }[]>(
      "get_community_members", { communityId },
    ),
  /**
   * Architecture §32 Phase 5 W15 — upload + cache the local user's
   * per-community avatar. Compresses to 128×128 WebP on the backend
   * and returns the BLAKE3 hex hash of the compressed bytes; pass the
   * hash to `updateCommunityProfile` as `avatarRef`.
   */
  setCommunityAvatar: (communityId: string, bytes: number[]) =>
    invoke<string>("set_community_avatar", { communityId, bytes }),
  /** Same as above for banner (compressed to 600×200). */
  setCommunityBanner: (communityId: string, bytes: number[]) =>
    invoke<string>("set_community_banner", { communityId, bytes }),
  /**
   * Resolve a stored avatar/banner blob to a `data:image/webp;base64,…`
   * URL ready to drop into an `<img src=…>`. Returns `null` if the
   * hash isn't cached locally yet (peer fetch rides Lost Cargo and
   * isn't part of this command).
   */
  getCommunityAvatarDataUrl: (communityId: string, hash: string) =>
    invoke<string | null>("get_community_avatar_data_url", { communityId, hash }),
  updateCommunityProfile: (
    communityId: string,
    bio: string | null,
    pronouns: string | null,
    themeColor: number | null,
    badges: string[],
    avatarRef: string | null,
    bannerRef: string | null,
  ) => invoke<void>("update_community_profile", {
    communityId,
    bio,
    pronouns,
    themeColor,
    badges,
    avatarRef,
    bannerRef,
  }),
  createCommunity: (name: string) =>
    invoke<string>("create_community", { name }),
  joinCommunity: (communityId: string, inviteCode?: string) =>
    invoke<void>("join_community", { communityId, inviteCode: inviteCode ?? null }),
  createChannel: (
    communityId: string,
    name: string,
    channelType: string,
    categoryId?: string,
    parentVoiceChannelId?: string,
  ) =>
    invoke<string>("create_channel", {
      communityId,
      name,
      channelType,
      categoryId: categoryId ?? null,
      parentVoiceChannelId: parentVoiceChannelId ?? null,
    }),
  sendChannelMessage: (channelId: string, body: string, replyToId?: string) =>
    invoke<SendChannelMessageResult>("send_channel_message", { channelId, body, replyToId: replyToId ?? null }),
  editChannelMessage: (channelId: string, messageId: string, newBody: string) =>
    invoke<void>("edit_channel_message", { channelId, messageId, newBody }),
  deleteChannelMessage: (channelId: string, messageId: string) =>
    invoke<void>("delete_channel_message", { channelId, messageId }),
  adminDeleteChannelMessage: (
    communityId: string,
    channelId: string,
    messageId: string,
    reason?: string,
  ) => invoke<void>("admin_delete_channel_message", {
    communityId,
    channelId,
    messageId,
    reason: reason ?? null,
  }),
  bulkDeleteChannelMessages: (
    communityId: string,
    channelId: string,
    messageIds: string[],
    reason?: string,
  ) => invoke<number>("bulk_delete_channel_messages", {
    communityId,
    channelId,
    messageIds,
    reason: reason ?? null,
  }),
  forwardChannelMessage: (
    sourceCommunityId: string,
    sourceChannelId: string,
    sourceMessageId: string,
    destCommunityId: string,
    destChannelId: string,
  ) => invoke<{ status: string; messageId: string }>("forward_channel_message", {
    sourceCommunityId,
    sourceChannelId,
    sourceMessageId,
    destCommunityId,
    destChannelId,
  }),
  uploadAttachment: (communityId: string, channelId: string, filePath: string) =>
    invoke<string>("upload_attachment", { communityId, channelId, filePath }),
  downloadAttachment: (
    communityId: string,
    channelId: string,
    attachmentId: string,
    savePath: string,
  ) => invoke<void>("download_attachment", { communityId, channelId, attachmentId, savePath }),
  pinAttachment: (communityId: string, attachmentId: string, pinned: boolean) =>
    invoke<void>("pin_attachment", { communityId, attachmentId, pinned }),
  sendVoiceMessage: (
    communityId: string,
    channelId: string,
    opusBytesB64: string,
    durationMs: number,
    waveformB64: string,
  ) => invoke<string>("send_voice_message", {
    communityId,
    channelId,
    opusBytesB64,
    durationMs,
    waveformB64,
  }),
  expandCommunitySegment: (communityId: string) =>
    invoke<number>("expand_community_segment", { communityId }),
  /**
   * Architecture §10.7 + §20.6 — read the community-wide policy
   * (rules text + raid-protection thresholds). Falls back to the §20.6
   * defaults (20 joins per 600 s) when no `CommunityPolicy` entry has
   * been merged yet.
   */
  getCommunityPolicy: (communityId: string) =>
    invoke<CommunityPolicy>("get_community_policy", { communityId }),
  /**
   * Architecture §10.7 + §20.6 — write a new `CommunityPolicy`
   * governance entry (admin-only). `policyText = null` clears the
   * rules text; the raid thresholds must both be > 0.
   */
  setCommunityPolicy: (
    communityId: string,
    policyText: string | null,
    maxJoinsPerInterval: number,
    joinIntervalSeconds: number,
  ) =>
    invoke<void>("set_community_policy", {
      communityId,
      policyText,
      maxJoinsPerInterval,
      joinIntervalSeconds,
    }),
  getChannelMessages: (channelId: string, limit: number) =>
    invoke<Message[]>("get_channel_messages", { channelId, limit }),
  getOlderChannelMessages: (communityId: string, channelId: string, beforeTimestamp: number, limit: number) =>
    invoke<Message[]>("get_older_channel_messages", { communityId, channelId, beforeTimestamp, limit }),
  createPoll: (
    communityId: string,
    channelId: string,
    messageId: string,
    question: string,
    answers: string[],
    multiSelect: boolean,
    durationSeconds?: number,
  ) => invoke<string>("create_poll", {
    communityId,
    channelId,
    messageId,
    question,
    answers,
    multiSelect,
    durationSeconds: durationSeconds ?? null,
  }),
  votePoll: (communityId: string, channelId: string, pollId: string, selectedAnswers: number[]) =>
    invoke<void>("vote_poll", { communityId, channelId, pollId, selectedAnswers }),
  closePoll: (communityId: string, channelId: string, pollId: string) =>
    invoke<void>("close_poll", { communityId, channelId, pollId }),
  getPollResults: (communityId: string, channelId: string, pollId: string) =>
    invoke<number[]>("get_poll_results", { communityId, channelId, pollId }),
  uploadEmoji: (communityId: string, name: string, bytes: number[], animated: boolean) =>
    invoke<string>("upload_emoji", { communityId, name, bytes, animated }),
  uploadSticker: (
    communityId: string,
    name: string,
    bytes: number[],
    animated: boolean,
    tags?: string[],
  ) =>
    invoke<string>("upload_sticker", {
      communityId,
      name,
      bytes,
      animated,
      tags: tags ?? null,
    }),
  /**
   * Architecture §18.3 — upload a soundboard clip. The frontend must
   * decode the OGG/MP3 (e.g. via Web Audio `decodeAudioData`) and pass
   * the resulting `durationSeconds`; the backend rejects clips longer
   * than 5.0 s. `volume` is `0.0..=1.0`; receivers multiply it into the
   * channel mix when the sound plays. `emoji` (single glyph) is shown
   * next to the sound in the picker.
   */
  uploadSoundboardSound: (
    communityId: string,
    name: string,
    bytes: number[],
    durationSeconds: number,
    volume: number,
    emoji?: string,
    tags?: string[],
  ) =>
    invoke<string>("upload_soundboard_sound", {
      communityId,
      name,
      bytes,
      tags: tags ?? null,
      durationSeconds,
      volume,
      emoji: emoji ?? null,
    }),
  /**
   * Architecture §10.9 — trigger a soundboard sound in the active voice
   * channel. Receivers fetch the (already-cached) audio asset and play
   * it locally at the uploader's normalised volume.
   */
  playSoundboard: (communityId: string, channelId: string, expressionId: string) =>
    invoke<void>("play_soundboard", { communityId, channelId, expressionId }),
  deleteEmoji: (communityId: string, expressionId: string) =>
    invoke<void>("delete_emoji", { communityId, expressionId }),
  listExpressions: (communityId: string) =>
    invoke<ExpressionInfo[]>("list_expressions", { communityId }),
  listAutoModRules: (communityId: string) =>
    invoke<AutoModRuleInfo[]>("list_automod_rules", { communityId }),
  setAutoModRule: (
    communityId: string,
    ruleId: string | null,
    name: string,
    enabled: boolean,
    keywords: string[],
    regexPatterns: string[],
    action: "block_locally" | "blur_content" | "alert_moderators",
  ) => invoke<string>("set_automod_rule", {
    communityId,
    ruleId,
    name,
    enabled,
    keywords,
    regexPatterns,
    action,
  }),
  deleteAutoModRule: (communityId: string, ruleId: string) =>
    invoke<void>("delete_automod_rule", { communityId, ruleId }),
  removeCommunityMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("remove_community_member", { communityId, pseudonymKey }),
  leaveCommunity: (communityId: string) =>
    invoke<void>("leave_community", { communityId }),
  deleteChannel: (communityId: string, channelId: string) =>
    invoke<void>("delete_channel", { communityId, channelId }),
  renameChannel: (communityId: string, channelId: string, newName: string) =>
    invoke<void>("rename_channel", { communityId, channelId, newName }),
  /**
   * Architecture §32 Phase 5 Week 15 — community info edit. Each
   * argument is null to leave the field unchanged. Backend reads
   * current `governance.metadata`, overrides only the supplied fields,
   * and re-emits a `CommunityMeta` LWW entry so unchanged values
   * (e.g. `iconHash`) are not nuked when only `name` is edited.
   * `iconHash`/`bannerHash` are BLAKE3 hex hashes returned by
   * `setCommunityAvatar`/`setCommunityBanner`.
   */
  updateCommunityInfo: (
    communityId: string,
    name: string | null,
    description: string | null,
    iconHash: string | null = null,
    bannerHash: string | null = null,
  ) =>
    invoke<void>("update_community_info", {
      communityId,
      name,
      description,
      iconHash,
      bannerHash,
    }),
  banMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("ban_member", { communityId, pseudonymKey }),
  unbanMember: (communityId: string, pseudonymKey: string) =>
    invoke<void>("unban_member", { communityId, pseudonymKey }),
  getBanList: (communityId: string) =>
    invoke<{ pseudonymKey: string; displayName: string; bannedAt: number }[]>(
      "get_ban_list", { communityId },
    ),
  rotateMek: (communityId: string) =>
    invoke<void>("rotate_mek", { communityId }),
  setChannelNotificationLevel: (
    communityId: string,
    channelId: string,
    level: "all" | "mentions" | "nothing",
  ) => invoke<void>("set_channel_notification_level", { communityId, channelId, level }),
  /**
   * Architecture §17.1 tier 2 — community-wide default notification
   * level (broadcast as a `CommunityNotificationDefault` governance
   * entry). Per-channel overrides still win locally per §17.1.
   */
  setCommunityDefaultNotificationLevel: (
    communityId: string,
    level: "all" | "mentions" | "nothing",
  ) => invoke<void>("set_community_default_notification_level", { communityId, level }),
  /** Read the current community default notification level (or `null`). */
  getCommunityDefaultNotificationLevel: (communityId: string) =>
    invoke<"all" | "mentions" | "nothing" | null>(
      "get_community_default_notification_level",
      { communityId },
    ),
  /**
   * Architecture §32 Phase 7 Week 25 — set the notification sound for
   * a channel (`channelId` non-empty) or for the community default
   * (`channelId = ""`). `soundRef = null` removes the override and
   * re-inherits from the next level up.
   */
  setNotificationSound: (
    communityId: string,
    channelId: string,
    soundRef: string | null,
  ) =>
    invoke<void>("set_notification_sound", {
      communityId,
      channelId,
      soundRef,
    }),
  /**
   * Resolve the effective notification sound for `(community, channel)`
   * using channel override → community default → null fallthrough.
   */
  getNotificationSound: (communityId: string, channelId: string) =>
    invoke<string | null>("get_notification_sound", {
      communityId,
      channelId,
    }),
  /**
   * Architecture §32 Phase 7 Week 25 — Do Not Disturb global toggle.
   * When `true`, suppresses every notification regardless of channel
   * level, mention status, or quiet-hours window.
   */
  setDoNotDisturb: (enabled: boolean) =>
    invoke<void>("set_do_not_disturb", { enabled }),
  getDoNotDisturb: () => invoke<boolean>("get_do_not_disturb"),
  /**
   * Architecture §28.8 line 3220 — IP-privacy toggle for outgoing link
   * previews. When false, this device skips the OpenGraph fetch so no
   * third-party server learns this device's IP. Receivers' previews
   * from other senders continue to render.
   */
  setLinkPreviewsEnabled: (enabled: boolean) =>
    invoke<void>("set_link_previews_enabled", { enabled }),
  getLinkPreviewsEnabled: () => invoke<boolean>("get_link_previews_enabled"),
  setQuietHours: (
    enabled: boolean,
    startHour: number,
    endHour: number,
    timezone: string,
  ) => invoke<void>("set_quiet_hours", {
    enabled,
    startHour,
    endHour,
    timezone,
  }),
  getQuietHours: () =>
    invoke<QuietHoursSettings>("get_quiet_hours"),

  // Reactions
  addReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("add_reaction", { communityId, channelId, messageId, emoji }),
  removeReaction: (communityId: string, channelId: string, messageId: string, emoji: string) =>
    invoke<void>("remove_reaction", { communityId, channelId, messageId, emoji }),

  // Pins
  pinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("pin_message", { communityId, channelId, messageId }),
  unpinMessage: (communityId: string, channelId: string, messageId: string) =>
    invoke<void>("unpin_message", { communityId, channelId, messageId }),
  getChannelPins: (communityId: string, channelId: string) =>
    invoke<{ messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]>(
      "get_channel_pins",
      { communityId, channelId },
    ),

  sendChannelTyping: (communityId: string, channelId: string) =>
    invoke<void>("send_channel_typing", { communityId, channelId }),

  updateCommunityPresence: (communityId: string, status: string, gameName?: string, gameId?: number, elapsedSeconds?: number, serverAddress?: string) =>
    invoke<void>("update_community_presence", { communityId, status, gameName, gameId, elapsedSeconds, serverAddress }),

  // Audit log
  getAuditLog: (communityId: string, beforeTimestamp?: number, limit: number = 50) =>
    invoke<{ action: string; actorPseudonym: string; target: string | null; details: string | null; timestamp: number }[]>(
      "get_audit_log",
      { communityId, beforeTimestamp, limit },
    ),

  // Categories
  createCategory: (communityId: string, name: string) =>
    invoke<{ categoryId: string }>("create_category", { communityId, name }),
  deleteCategory: (communityId: string, categoryId: string) =>
    invoke<void>("delete_category", { communityId, categoryId }),
  renameCategory: (communityId: string, categoryId: string, newName: string) =>
    invoke<void>("rename_category", { communityId, categoryId, newName }),
  moveChannel: (communityId: string, channelId: string, categoryId: string | null) =>
    invoke<void>("move_channel", { communityId, channelId, categoryId }),
  reorderCategories: (communityId: string, categoryIds: string[]) =>
    invoke<void>("reorder_categories", { communityId, categoryIds }),
  setChannelTopic: (communityId: string, channelId: string, topic: string) =>
    invoke<void>("set_channel_topic", { communityId, channelId, topic }),
  setChannelForumTags: (communityId: string, channelId: string, forumTags: string[]) =>
    invoke<void>("set_channel_forum_tags", { communityId, channelId, forumTags }),
  reorderChannels: (communityId: string, channelIds: string[]) =>
    invoke<void>("reorder_channels", { communityId, channelIds }),

  // Community invites
  createCommunityInvite: (communityId: string, maxUses?: number, expiresInSeconds?: number) =>
    invoke<{ code: string; governanceKey: string }>("create_community_invite", { communityId, maxUses: maxUses ?? null, expiresInSeconds: expiresInSeconds ?? null }),
  revokeCommunityInvite: (communityId: string, codeHash: string) =>
    invoke<void>("revoke_community_invite", { communityId, codeHash }),
  listCommunityInvites: (communityId: string) =>
    invoke<{ codeHash: string; createdBy: string; maxUses: number | null; uses: number; expiresAt: number | null; createdAt: number; code?: string }[]>(
      "list_community_invites", { communityId }
    ),

  // Roles
  getRoles: (communityId: string) =>
    invoke<{ id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean; exclusionGroup?: string }[]>(
      "get_roles", { communityId },
    ),
  createRole: (
    communityId: string,
    name: string,
    color: number,
    permissions: string,
    hoist: boolean,
    mentionable: boolean,
    selfAssignable: boolean,
    /** Architecture §19.4 — when set, the member can hold at most one role per group. */
    exclusionGroup?: string,
  ) =>
    invoke<number>("create_role", {
      communityId,
      name,
      color,
      permissions,
      hoist,
      mentionable,
      selfAssignable,
      exclusionGroup: exclusionGroup ?? null,
    }),
  editRole: (
    communityId: string,
    roleId: number,
    name: string | null,
    color: number | null,
    permissions: string | null,
    position: number | null,
    hoist: boolean | null,
    mentionable: boolean | null,
    selfAssignable: boolean | null,
    /**
     * Architecture §19.4 — pass `{ kind: "set", value: "..." }` to set,
     * `{ kind: "clear" }` to clear, or omit to leave unchanged.
     */
    exclusionGroup?: ExclusionGroupEdit,
  ) =>
    invoke<void>("edit_role", {
      communityId,
      roleId,
      name,
      color,
      permissions,
      position,
      hoist,
      mentionable,
      selfAssignable,
      exclusionGroup: exclusionGroup ?? null,
    }),
  deleteRole: (communityId: string, roleId: number) =>
    invoke<void>("delete_role", { communityId, roleId }),
  assignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("assign_role", { communityId, pseudonymKey, roleId }),
  unassignRole: (communityId: string, pseudonymKey: string, roleId: number) =>
    invoke<void>("unassign_role", { communityId, pseudonymKey, roleId }),
  selfAssignRole: (communityId: string, roleId: number) =>
    invoke<void>("self_assign_role", { communityId, roleId }),
  selfUnassignRole: (communityId: string, roleId: number) =>
    invoke<void>("self_unassign_role", { communityId, roleId }),
  timeoutMember: (communityId: string, pseudonymKey: string, durationSeconds: number, reason: string | null) =>
    invoke<void>("timeout_member", { communityId, pseudonymKey, durationSeconds, reason }),
  removeTimeout: (communityId: string, pseudonymKey: string) =>
    invoke<void>("remove_timeout", { communityId, pseudonymKey }),
  setChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string, allow: number, deny: number) =>
    invoke<void>("set_channel_overwrite", { communityId, channelId, targetType, targetId, allow, deny }),
  deleteChannelOverwrite: (communityId: string, channelId: string, targetType: string, targetId: string) =>
    invoke<void>("delete_channel_overwrite", { communityId, channelId, targetType, targetId }),
  setSlowmode: (communityId: string, channelId: string, seconds: number) =>
    invoke<void>("set_slowmode", { communityId, channelId, seconds }),

  // Threads
  /**
   * Architecture §32 Phase 6 W19 — `autoArchiveSeconds` must be one of
   * 3600 (1h), 86400 (24h), 259200 (3d), or 604800 (7d). Omit to let
   * the backend pick the per-thread-type default.
   */
  createThread: (
    communityId: string,
    channelId: string,
    name: string,
    starterMessageId: string,
    forumTag?: string | null,
    autoArchiveSeconds?: number,
  ) =>
    invoke<string>("create_thread", {
      communityId,
      channelId,
      name,
      starterMessageId,
      forumTag: forumTag ?? null,
      autoArchiveSeconds: autoArchiveSeconds ?? null,
    }),
  getChannelThreads: (communityId: string, channelId: string) =>
    invoke<{ id: string; channelId: string; name: string; starterMessageId: string; creatorPseudonym: string; forumTag?: string | null; createdAt: number; archived: boolean; autoArchiveSeconds: number; lastMessageAt: number; messageCount: number }[]>(
      "get_channel_threads", { communityId, channelId }),
  getActiveThreads: (communityId: string, channelId: string) =>
    invoke<{ id: string; channelId: string; name: string; starterMessageId: string; creatorPseudonym: string; forumTag?: string | null; createdAt: number; archived: boolean; autoArchiveSeconds: number; lastMessageAt: number; messageCount: number }[]>(
      "get_active_threads", { communityId, channelId }),
  sendThreadMessage: (communityId: string, threadId: string, body: string) =>
    invoke<void>("send_thread_message", { communityId, threadId, body }),
  getThreadMessages: (communityId: string, threadId: string, limit: number, beforeTimestamp?: number) =>
    invoke<Message[]>("get_thread_messages", { communityId, threadId, limit, beforeTimestamp: beforeTimestamp ?? null }),
  archiveThread: (communityId: string, threadId: string) =>
    invoke<void>("archive_thread", { communityId, threadId }),
  unarchiveThread: (communityId: string, threadId: string) =>
    invoke<void>("unarchive_thread", { communityId, threadId }),

  // Community Events (architecture §21)
  createEvent: (communityId: string, request: CreateEventRequest) =>
    invoke<string>("create_event", { communityId, request }),
  editEvent: (communityId: string, eventId: string, title?: string, description?: string, startTime?: number, endTime?: number, channelId?: string, maxAttendees?: number) =>
    invoke<void>("edit_event", { communityId, eventId, title: title ?? null, description: description ?? null, startTime: startTime ?? null, endTime: endTime ?? null, channelId: channelId ?? null, maxAttendees: maxAttendees ?? null }),
  deleteEvent: (communityId: string, eventId: string) =>
    invoke<void>("delete_event", { communityId, eventId }),
  cancelEvent: (communityId: string, eventId: string) =>
    invoke<void>("cancel_event", { communityId, eventId }),
  rsvpEvent: (communityId: string, eventId: string, status: string) =>
    invoke<void>("rsvp_event", { communityId, eventId, status }),
  setEventRsvp: (communityId: string, eventId: string, status: string) =>
    invoke<void>("set_event_rsvp", { communityId, eventId, status }),
  listEventAttendees: (communityId: string, eventId: string) =>
    invoke<{ pseudonymKey: string; status: string }[]>("list_event_attendees", { communityId, eventId }),
  getEvents: (communityId: string) =>
    invoke<EventInfo[]>("get_events", { communityId }),

  // Game Servers
  addGameServer: (communityId: string, gameId: string, label: string, address: string) =>
    invoke<string>("add_game_server", { communityId, gameId, label, address }),
  removeGameServer: (communityId: string, serverId: string) =>
    invoke<void>("remove_game_server", { communityId, serverId }),
  getGameServers: (communityId: string) =>
    invoke<{ id: string; gameId: string; label: string; address: string; addedBy: string; createdAt: number }[]>(
      "get_game_servers", { communityId }),

  // Unread tracking
  markChannelRead: (communityId: string, channelId: string, lastMessageId: string) =>
    invoke<void>("mark_channel_read", { communityId, channelId, lastMessageId }),
  getUnreadCounts: (communityId: string) =>
    invoke<{ channelId: string; unreadCount: number }[]>("get_unread_counts", { communityId }),

  // Voice
  joinVoiceChannel: (channelId: string, communityId?: string) =>
    invoke<void>("join_voice_channel", { channelId, communityId: communityId ?? null }),
  leaveVoice: () => invoke<void>("leave_voice"),
  requestToSpeak: (communityId: string, channelId: string) =>
    invoke<void>("request_to_speak", { communityId, channelId }),
  getStageHandRaises: (communityId: string, channelId: string) =>
    invoke<string[]>("get_stage_hand_raises", { communityId, channelId }),
  respondToSpeakRequest: (communityId: string, channelId: string, requesterPseudonym: string, granted: boolean) =>
    invoke<void>("respond_to_speak_request", { communityId, channelId, requesterPseudonym, granted }),
  setMute: (muted: boolean) => invoke<void>("set_mute", { muted }),
  setDeafen: (deafened: boolean) => invoke<void>("set_deafen", { deafened }),
  /**
   * Architecture §10 — moderator action: server-mute another member
   * in a community voice channel. Backend gates on `MUTE_MEMBERS`
   * and broadcasts a `Control::VoiceMute` envelope through the
   * gossip mesh; receivers honour it locally if the actor still has
   * the perm in CRDT-merged governance.
   */
  serverMuteMember: (
    communityId: string,
    channelId: string,
    targetPseudonym: string,
    muted: boolean,
  ) =>
    invoke<void>("server_mute_member", {
      communityId,
      channelId,
      targetPseudonym,
      muted,
    }),
  /**
   * Architecture §10 — moderator action: server-deafen another
   * member. Backend gates on `DEAFEN_MEMBERS` and broadcasts a
   * `Control::VoiceDeafen` envelope.
   */
  serverDeafenMember: (
    communityId: string,
    channelId: string,
    targetPseudonym: string,
    deafened: boolean,
  ) =>
    invoke<void>("server_deafen_member", {
      communityId,
      channelId,
      targetPseudonym,
      deafened,
    }),
  listAudioDevices: () => invoke<AudioDevices>("list_audio_devices"),
  setAudioDevices: (inputDevice: string | null, outputDevice: string | null) =>
    invoke<void>("set_audio_devices", { inputDevice, outputDevice }),
  setVoiceMode: (mode: string, hostPseudonym?: string) =>
    invoke<void>("set_voice_mode", { mode, hostPseudonym: hostPseudonym ?? null }),

  // Plan §Failure 5 — direct call offer/accept handshake.
  startDmCall: (peerPublicKey: string, video: boolean) =>
    invoke<string>("start_dm_call", { peerPublicKey, video }),
  acceptDmCall: (callId: string) =>
    invoke<void>("accept_dm_call", { callId }),
  declineDmCall: (callId: string, reason?: string) =>
    invoke<void>("decline_dm_call", { callId, reason: reason ?? null }),
  /// C2 hangup — end an Active call (post-handshake), distinct from
  /// declineDmCall (which rejects a CallOffer before accepting).
  endDmCall: (callId: string, reason?: string) =>
    invoke<void>("end_dm_call", { callId, reason: reason ?? null }),
  getMissedCalls: () => invoke<MissedCallRow[]>("get_missed_calls"),

  // Status
  setStatus: (status: string) => invoke<void>("set_status", { status }),
  setNickname: (nickname: string) =>
    invoke<void>("set_nickname", { nickname }),
  setAvatar: (avatarData: number[]) =>
    invoke<void>("set_avatar", { avatarData }),
  getAvatar: (publicKey: string) =>
    invoke<number[] | null>("get_avatar", { publicKey }),
  setStatusMessage: (message: string) =>
    invoke<void>("set_status_message", { message }),

  // Game
  getGameStatus: () => invoke<GameStatus | null>("get_game_status"),
  getGameName: (gameId: number) => invoke<string | null>("get_game_name", { gameId }),
  launchGameToServer: (gameId: number, serverAddress: string) =>
    invoke<void>("launch_game_to_server", { gameId, serverAddress }),

  // Settings
  getPreferences: () => invoke<Preferences>("get_preferences"),
  setPreferences: (prefs: Preferences) =>
    invoke<void>("set_preferences", { prefs }),
  checkForUpdates: () => invoke<boolean>("check_for_updates"),

  // Windows
  showBuddyList: () => invoke<void>("show_buddy_list"),
  openChatWindow: (publicKey: string, displayName: string) =>
    invoke<void>("open_chat_window", { publicKey, displayName }),
  openSettingsWindow: (tab?: string) => invoke<void>("open_settings_window", { tab: tab ?? null }),
  openCommunityWindow: (communityId: string, communityName: string) =>
    invoke<void>("open_community_window", { communityId, communityName }),
  openProfileWindow: (publicKey: string, displayName: string) =>
    invoke<void>("open_profile_window", { publicKey, displayName }),
  getNetworkStatus: () => invoke<NetworkStatus>("get_network_status"),

  // Onboarding & Welcome Screen
  getOnboardingConfig: (communityId: string) =>
    invoke<OnboardingConfig>("get_onboarding_config", { communityId }),
  setOnboardingConfig: (communityId: string, config: OnboardingConfig) =>
    invoke<void>("set_onboarding_config", { communityId, config }),
  getWelcomeScreen: (communityId: string) =>
    invoke<WelcomeScreen>("get_welcome_screen", { communityId }),
  setWelcomeScreen: (communityId: string, screen: WelcomeScreen) =>
    invoke<void>("set_welcome_screen", { communityId, screen }),
  /**
   * Architecture §19.2 step 3 — `acknowledgedRules` must be `true`
   * when the merged `OnboardingConfig.mode === "gated"`, otherwise the
   * backend rejects the submission with a clear error. For
   * `default` / `guided` modes the flag is ignored.
   */
  submitOnboardingAnswers: (
    communityId: string,
    answers: OnboardingAnswer[],
    acknowledgedRules?: boolean,
  ) =>
    invoke<void>("submit_onboarding_answers", {
      communityId,
      answers,
      acknowledgedRules: acknowledgedRules ?? null,
    }),
  /** Plan §Failure 8 — persist `community_members.onboarding_complete = 1` for
   *  the local user so the wizard does not re-show on next launch. */
  markOnboardingComplete: (communityId: string) =>
    invoke<void>("mark_onboarding_complete", { communityId }),
  debugGossipState: (communityId: string) =>
    invoke<GossipDiagnostics>("debug_gossip_state", { communityId }),

  // Direct messages (architecture §27)
  listDms: () => invoke<DmConversation[]>("list_dms"),
  startDm: (bobPublicKey: string, alicePseudonym: string) =>
    invoke<string>("start_dm", { bobPublicKey, alicePseudonym }),
  acceptDmInvite: (recordKey: string) =>
    invoke<void>("accept_dm_invite", { recordKey }),
  declineDmInvite: (recordKey: string) =>
    invoke<void>("decline_dm_invite", { recordKey }),
  sendDmMessage: (recordKey: string, body: string) =>
    invoke<void>("send_dm_message", { recordKey, body }),
  getDmMessages: (recordKey: string, limit: number) =>
    invoke<DmMessageRecord[]>("get_dm_messages", { recordKey, limit }),
  openDmWindow: (recordKey: string, titleHint: string) =>
    invoke<void>("open_dm_window", { recordKey, titleHint }),

  // Strand Relay Network (architecture §13)
  volunteerRelay: (friendPublicKey: string) =>
    invoke<void>("volunteer_relay", { friendPublicKey }),
  revokeRelay: (friendPublicKey: string) =>
    invoke<void>("revoke_relay", { friendPublicKey }),
  listReceivedRelayOffers: () =>
    invoke<string[]>("list_received_relay_offers"),
  listVolunteeredRelayFriends: () =>
    invoke<string[]>("list_volunteered_relay_friends"),

  // Mobile Push Relay (architecture §17.3)
  registerWithPushRelay: (
    relayPseudonym: string,
    devicePushToken: string,
    platform: "fcm" | "apns" | "self",
    recordKeys: string[],
  ) =>
    invoke<void>("register_with_push_relay", {
      relayPseudonym,
      devicePushToken,
      platform,
      recordKeys,
    }),
  unregisterWithPushRelay: (relayPseudonym: string) =>
    invoke<void>("unregister_with_push_relay", { relayPseudonym }),
  listPushRelayRegistrations: () =>
    invoke<[string, string, string][]>("list_push_relay_registrations"),
  searchMessages: (request: MessageSearch) =>
    invoke<SearchResult>("search_messages", { request }),
  getCommunityAnalytics: (communityId: string) =>
    invoke<CommunityAnalytics>("get_community_analytics", { communityId }),
  ensurePersonalSyncRecord: () => invoke<string>("ensure_personal_sync_record"),
  startPairingSession: () => invoke<PairingSession>("start_pairing_session"),
  generatePairingQrSvg: () => invoke<PairingQrPayload>("generate_pairing_qr_svg"),
  acceptPairingCode: (
    pairingCode: string,
    pairingSaltHex: string,
    existingDeviceRouteBlobHex: string,
    displayName: string,
  ) =>
    invoke<PairingAccept>("accept_pairing_code", {
      pairingCode,
      pairingSaltHex,
      existingDeviceRouteBlobHex,
      displayName,
    }),
  readSyncManifest: () => invoke<SyncManifest | null>("read_sync_manifest"),
  writeSyncManifest: (manifest: SyncManifest) =>
    invoke<void>("write_sync_manifest", { manifest }),
  readSyncReadState: () => invoke<SyncReadState>("read_sync_read_state"),
  writeSyncReadState: (readState: SyncReadState) =>
    invoke<SyncReadState>("write_sync_read_state", { readState }),
  readSyncPreferences: () => invoke<SyncPreferences>("read_sync_preferences"),
  writeSyncPreferences: (preferences: SyncPreferences) =>
    invoke<SyncPreferences>("write_sync_preferences", { preferences }),
  readPairedDevices: () => invoke<DeviceList>("read_paired_devices"),
  writePairedDevices: (devices: DeviceList) =>
    invoke<DeviceList>("write_paired_devices", { devices }),
  fetchLinkPreview: (
    communityId: string,
    channelId: string,
    messageId: string,
    url: string,
  ) =>
    invoke<LinkPreview>("fetch_link_preview", {
      communityId,
      channelId,
      messageId,
      url,
    }),
  runBackgroundSync: () =>
    invoke<BackgroundSyncReport>("run_background_sync"),
  /**
   * Send one VP9-encoded frame chunk (from the WebCodecs VideoEncoder
   * output) into the community video stream. The backend MEK-encrypts,
   * fragments to ≤28 KB, attaches FEC parity for keyframes, signs each
   * fragment, and broadcasts via gossip. Returns the number of
   * fragments dispatched (data + parity).
   */
  sendVideoFrame: (
    communityId: string,
    channelId: string,
    request: SendVideoFrameRequest,
  ) =>
    invoke<number>("send_video_frame", { communityId, channelId, request }),
  /**
   * Derive the deterministic 16-byte stream_id the encoder must stamp
   * into each outbound `VideoFragment`. `trackLabel` distinguishes
   * concurrent streams from the same member — `"camera"` for the
   * webcam track from `getUserMedia()`, `"screen"` for the
   * `getDisplayMedia()` track in the same call (architecture §10.6).
   * Returns lowercase hex.
   */
  deriveVideoStreamId: (
    communityId: string,
    channelId: string,
    trackLabel: VideoTrackLabel,
  ) =>
    invoke<string>("derive_video_stream_id", {
      communityId,
      channelId,
      trackLabel,
    }),
  /**
   * Architecture §10.6 — interim default media capabilities (480p @
   * 15fps, VP9 only) for clients that don't introspect their hardware.
   * The video send-side init in `VoicePanel.tsx::startVideo()` seeds
   * its WebCodecs `VideoEncoderConfig` with these values when the
   * browser's `MediaCapabilities` API isn't available, then advertises
   * the result in `MediaCapabilities` envelopes so peers can size
   * their VP9 bitrate to the slowest receiver.
   */
  defaultMediaCapabilities: () =>
    invoke<{ maxPixelCount: number; maxFps: number; codecs: string[] }>(
      "default_media_capabilities",
    ),
  /**
   * Architecture §10.6 line 4081 — receiver acks frames roughly every
   * 500 ms with measured downstream kbps + loss so senders can adapt
   * VP9 bitrate. `lossQ8` is fixed-point 0..=255 (0 = perfect).
   */
  sendVideoFrameAck: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
    lastFrameSeq: number,
    kbps: number,
    lossQ8: number,
  ) =>
    invoke<void>("send_video_frame_ack", {
      communityId,
      channelId,
      streamIdHex,
      lastFrameSeq,
      kbps,
      lossQ8,
    }),
  /**
   * Architecture §10.6 line 4081 — receiver lost too many inter-frames
   * and asks the sender to mark the next frame as a keyframe.
   */
  sendVideoKeyframeRequest: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
  ) =>
    invoke<void>("send_video_keyframe_request", {
      communityId,
      channelId,
      streamIdHex,
    }),
  /**
   * Architecture §10.6 line 4082 — out-of-band bandwidth advertisement
   * when network conditions change between frames (Wi-Fi → cellular).
   */
  sendVideoBandwidthEstimate: (
    communityId: string,
    channelId: string,
    kbps: number,
    windowSecs: number,
    lossQ8: number,
  ) =>
    invoke<void>("send_video_bandwidth_estimate", {
      communityId,
      channelId,
      kbps,
      windowSecs,
      lossQ8,
    }),
  /**
   * Architecture §10.6 Phase 6 Week 22 — broadcast that the active
   * video relay for `(channelId, streamIdHex)` has changed. Receivers
   * re-attach decoders to the new relay's stream and reset reassembly
   * buffers. `relayHostPseudonym = null` reverts to direct mesh
   * delivery.
   */
  notifyVideoTopologyChange: (
    communityId: string,
    channelId: string,
    streamIdHex: string,
    relayHostPseudonym: string | null,
    reason: VideoTopologyReason,
  ) =>
    invoke<void>("notify_video_topology_change", {
      communityId,
      channelId,
      streamIdHex,
      relayHostPseudonym,
      reason,
    }),
};

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

export function avatarDataUrl(base64: string | null | undefined): string | undefined {
  if (!base64) return undefined;
  return `data:image/webp;base64,${base64}`;
}
