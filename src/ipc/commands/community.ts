import { invoke } from "../invoke";
import type {
  AutoModRuleInfo, CommunityPolicy, ExpressionInfo, Message, QuietHoursSettings, SendChannelMessageResult,
} from "./types";

export const communityCommands = {
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
    idempotencyKey?: string,
  ) =>
    invoke<string>("create_channel", {
      communityId,
      name,
      channelType,
      categoryId: categoryId ?? null,
      parentVoiceChannelId: parentVoiceChannelId ?? null,
      // Phase 8 — idempotency key dedupes click-spam. Channel creation
      // allocates a fresh DHT record + writes a unique ChannelCreated
      // entry, so double-click without idempotency creates duplicates.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
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
  removeCommunityMember: (
    communityId: string,
    pseudonymKey: string,
    idempotencyKey?: string,
  ) =>
    invoke<void>("remove_community_member", {
      communityId,
      pseudonymKey,
      // Phase 8 — idempotency key dedupes click-spam.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
  leaveCommunity: (communityId: string) =>
    invoke<void>("leave_community", { communityId }),
  deleteChannel: (communityId: string, channelId: string, idempotencyKey?: string) =>
    invoke<void>("delete_channel", {
      communityId,
      channelId,
      // Phase 8 — idempotency key dedupes click-spam.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
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
  rotateMek: (communityId: string, idempotencyKey?: string) =>
    invoke<void>("rotate_mek", {
      communityId,
      // Phase 8 — idempotency key dedupes click-spam. Without it, a
      // double-rotate desyncs MEK generations across mesh peers.
      idempotencyKey: idempotencyKey ?? crypto.randomUUID(),
    }),
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
};
