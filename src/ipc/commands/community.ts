import { hostInvoke, invoke } from "../invoke";
import type { ImageKind } from "./system";
import type { KeyStatus, OpPublicationState } from "../channels/community_subscription_events";

/// A voice message as the Host decoded it: the Ogg Opus audio and the
/// duration and waveform read from it (base64 fields).
/** This device's standing in a community (`DeviceStatus`, plan E3.2 / ADR 0022 rule 4):
 *  `needsReenrol` while it holds no key endorsed for our current member record. */
export type DeviceStatus = "enrolled" | "needsReenrol";

export interface VoiceMessageAudioView {
  audio: string;
  durationMs: number;
  waveform: string;
}
import type {
  AutoModRuleInfo, CommunityPolicy, ExpressionInfo, Message, QuietHoursSettings, SendChannelMessageResult,
} from "./types";
import type { CommunityBanView, MemberGroup, TargetModeration } from "./dto";

/** `SuccessorOffer` (rekindle-types `dto::community`): a successor both
 *  sides link, as `communitySuccessorOffered` carries it. */
export interface SuccessorOffer {
  community: string;
  newAnchor: string;
  signer: string;
  reason: "recreate" | "transfer";
  creators: string[];
}

/** `SuccessorLinkRequested` (rekindle-types `dto::community`): a
 *  transfer's link request asking us, as `successorLinkRequested` carries it. */
export interface SuccessorLinkRequested {
  community: string;
  request: string;
  recipient: string;
  newAnchor: string;
}

/** `CommunitySuccessors` (rekindle-types `dto::community`): the successor
 *  snapshot, offers this session and link requests asking us now. */
export interface CommunitySuccessors {
  offers: SuccessorOffer[];
  linkRequests: SuccessorLinkRequested[];
}

/** `CommunityCreated` (rekindle-types `dto::community`). */
export interface CommunityCreated {
  anchor: string;
  registryKey: string;
  name: string;
  pendingCoCreators: string[];
}

/// The held co-creator invites and our pending creations, as the Host
/// holds them (`CoCreationSnapshot`, plan E3.2 S7s.3g).
export interface CoCreationSnapshot {
  invites: { anchor: string; peerKey: string; communityName: string }[];
  creations: {
    anchor: string;
    name: string;
    coCreators: { peerKey: string; status: string }[];
  }[];
}

export const communityCommands = {
  // Community
  getCommunities: () =>
    hostInvoke<{ anchor: string; name: string; description: string; memberCount: number; channelCount: number; ourPseudonym: string }[]>("get_communities"),
  getCommunityDetails: () =>
    hostInvoke<{
      id: string;
      name: string;
      description: string | null;
      iconHash?: string | null;
      bannerHash?: string | null;
      channels: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null; locked: boolean; lockedReason: string }[];
      categories?: { id: string; name: string; sortOrder: number }[];
      myRole: string | null;
      myRoleIds: string[];
      myPermissions: string;
      myChannelPermissions: Record<string, string>;
      roles: { id: string; name: string; color: number; tagColours: { dark: number; light: number }; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean; exclusionGroup?: string }[];
      myPseudonymKey: string | null;
      mekGeneration: number;
      onboardingComplete: boolean;
      /** We hold a creator seat now (`CommunityView::we_are_creator`); re-read on `effectivePermissionsChanged` naming us. */
      weAreCreator: boolean;
      /** Our name here, never empty: our per-community name, else our short fingerprint (plan E3.4 S6). */
      myDisplayName: string;
      /** Our per-community name as set; absent when unset (the set-name form's prefill). */
      myCardName?: string;
      /** Our card's custom status; absent when none. */
      myCustomStatus?: string;
      myBio?: string | null;
      myPronouns?: string | null;
      myThemeColor?: number | null;
      myBadges?: string[];
      removed?: {
    banned: boolean;
    by: string | null;
    byName: string | null;
    reason?: string | null;
  } | null;
      /** Our ops not yet landed, by op hash (`CommunityView::op_publications`). */
      opPublications?: Record<string, OpPublicationState>;
      /** Whether this device may write here (`CommunityView::device_status`, ADR 0022 rule 4). */
      deviceStatus: DeviceStatus;
      /** Our community key, as the Host states it (`CommunityView::key_status`); changes arrive as `keyStatus`. */
      keyStatus: KeyStatus;
    }[]>("get_community_details"),
  getCommunityMembers: (communityId: string) =>
    hostInvoke<{
      pseudonymKey: string;
      /** Never empty: the member's per-community name, else their short fingerprint (plan E3.4 S6). */
      displayName: string;
      /** Always the short pseudonym fingerprint ("a1b2 c3d4"), for a secondary line or tooltip. */
      fingerprint: string;
      /** Their card's custom status; absent when none. */
      customStatus?: string;
      roleIds: string[];
      displayRole: string;
      status: string;
      timeoutUntil: number | null;
      bio?: string | null;
      pronouns?: string | null;
      themeColor?: number | null;
      badges?: string[];
      avatarRef?: string | null;
      bannerRef?: string | null;
      location?: { kind: "text" | "voice"; channelId: string } | null;
      lastActive: number;
      group: MemberGroup;
      canModerateTarget: TargetModeration;
      outranks: boolean;
    }[]>(
      "get_community_members", { communityId },
    ),
  /**
   * Architecture §32 Phase 5 W15 — upload + cache the local user's
   * per-community avatar. Compresses to 128×128 WebP on the backend
   * and returns the BLAKE3 hex hash of the compressed bytes; pass the
   * hash to `updateCommunityProfile` as `avatarRef`.
   */
  setCommunityAvatar: (communityId: string, bytes: number[], kind: ImageKind) =>
    hostInvoke<string>("set_community_avatar", { communityId, bytes, kind }),
  /** Same as above for banner (compressed to 600×200). */
  setCommunityBanner: (communityId: string, bytes: number[], kind: ImageKind) =>
    hostInvoke<string>("set_community_banner", { communityId, bytes, kind }),
  /**
   * Resolve a stored avatar/banner blob to a `data:image/webp;base64,…`
   * URL ready to drop into an `<img src=…>`. Returns `null` if the
   * hash isn't cached locally yet (peer fetch rides Lost Cargo and
   * isn't part of this command).
   */
  getCommunityAvatarDataUrl: (communityId: string, hash: string) =>
    hostInvoke<string | null>("get_community_avatar_data_url", { communityId, hash }),
  updateCommunityProfile: (
    communityId: string,
    bio: string | null,
    pronouns: string | null,
    themeColor: number | null,
    badges: string[] | null,
    avatarRef: string | null,
    bannerRef: string | null,
    customStatus: string | null,
  ) => hostInvoke<void>("update_community_profile", {
    communityId,
    bio,
    pronouns,
    themeColor,
    badges,
    avatarRef,
    bannerRef,
    customStatus,
  }),
  /** Our name in this community (`CommunitySetDisplayName`, the Host's review
   *  SetCommunityDisplayName); `""` clears it and the fingerprint shows. */
  setCommunityDisplayName: (communityId: string, displayName: string) =>
    hostInvoke<void>("set_community_display_name", { communityId, displayName }),
  createCommunity: (name: string) =>
    hostInvoke<CommunityCreated>(
      "create_community", { name },
    ).then((created) => created.anchor),
  /// Creates with friends named co-creators (plan E3.2 S7s.3): the
  /// genesis waits for each to accept, decline or be removed, so the
  /// result lists the co-creators still pending.
  createCommunityWithCoCreators: (name: string, coCreators: string[]) =>
    hostInvoke<CommunityCreated>("create_community", { name, coCreators }),
  /// Read once the window's event stream is live (plan VI1's pattern).
  getCoCreation: () => invoke<CoCreationSnapshot>("get_co_creation"),
  /// Re-creates the community under a new anchor naming it as
  /// predecessor, with us as sole creator, and offers it (plan E3.2 S7s.4);
  /// `continuing` (pseudonym hex) get a sealed invite, absent every member.
  publishCommunitySuccessor: (communityId: string, continuing?: string[]) =>
    hostInvoke<CommunityCreated>("publish_community_successor", { communityId, continuing }),
  /// Follows an offered successor with the invite sealed to us in its
  /// invites record; the old community is kept.
  followCommunitySuccessor: (communityId: string, newAnchor: string) =>
    hostInvoke<{ anchor: string; name: string }>("follow_community_successor", { communityId, newAnchor }),
  /// The successor snapshot (lead decisions (33), (46)): the offers this
  /// session and the link requests asking us, read after the event stream
  /// is live.
  getCommunitySuccessors: (communityId: string) =>
    hostInvoke<CommunitySuccessors>("get_community_successors", { communityId }),
  removeCoCreator: (communityId: string, identity: string) =>
    hostInvoke<void>("remove_co_creator", { communityId, identity }),
  cancelCommunityCreate: (communityId: string) =>
    hostInvoke<void>("cancel_community_create", { communityId }),
  answerCreatorInvite: (communityId: string, accept: boolean) =>
    hostInvoke<void>("answer_creator_invite", { communityId, accept }),
  /// Joins through the Host's review of the invite (its record-key
  /// fingerprint); returns the joined community's id.
  joinCommunity: (inviteUrl: string) =>
    hostInvoke<{ anchor: string; name: string }>("join_community", { inviteUrl })
      .then((joined) => joined.anchor),
  createChannel: (
    communityId: string,
    name: string,
    channelType: string,
    categoryId?: string,
    parentVoiceChannelId?: string,
  ) =>
    hostInvoke<{ id: string }>("create_channel", {
      communityId,
      name,
      channelType,
      categoryId: categoryId ?? null,
      parentVoiceChannelId: parentVoiceChannelId ?? null,
    }).then((created) => created.id),
  sendChannelMessage: (channelId: string, body: string, replyToId?: string) =>
    hostInvoke<SendChannelMessageResult>("send_channel_message", { channelId, body, replyToId: replyToId ?? null }),
  editChannelMessage: (channelId: string, messageId: string, newBody: string) =>
    hostInvoke<void>("edit_channel_message", { channelId, messageId, newBody }),
  deleteChannelMessage: (channelId: string, messageId: string) =>
    hostInvoke<void>("delete_channel_message", { channelId, messageId }),
  adminDeleteChannelMessage: (
    communityId: string,
    channelId: string,
    messageId: string,
    reason?: string,
  ) => hostInvoke<null>("admin_delete_channel_message", {
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
  ) => hostInvoke<number>("bulk_delete_channel_messages", {
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
  ) => hostInvoke<{ status: string; messageId: string }>("forward_channel_message", {
    sourceCommunityId,
    sourceChannelId,
    sourceMessageId,
    destCommunityId,
    destChannelId,
  }),
  /// Opens a native file picker; resolves to the new attachment id, or
  /// null if the user cancelled.
  uploadAttachment: (communityId: string, channelId: string) =>
    hostInvoke<string | null>("upload_attachment", { communityId, channelId }),
  /// Opens a native save dialog; resolves to false if the user cancelled.
  downloadAttachment: (communityId: string, channelId: string, attachmentId: string) =>
    hostInvoke<boolean>("download_attachment", { communityId, channelId, attachmentId }),
  /// A voice message's audio, fetched into the chunk cache if needed.
  getVoiceMessageAudio: (communityId: string, channelId: string, attachmentId: string) =>
    hostInvoke<VoiceMessageAudioView>("get_voice_message_audio", {
      communityId,
      channelId,
      attachmentId,
    }),
  /// Show a downloaded attachment in the OS file manager.
  revealDownloadedAttachment: (communityId: string, channelId: string, attachmentId: string) =>
    hostInvoke<unknown>("reveal_downloaded_attachment", { communityId, channelId, attachmentId }).then(
      () => undefined,
    ),
  pinAttachment: (communityId: string, attachmentId: string, pinned: boolean) =>
    hostInvoke<unknown>("pin_attachment", { communityId, attachmentId, pinned }).then(
      () => undefined,
    ),
  /// `pcmB64`: the recording as mono little-endian f32 PCM at 16 kHz; the
  /// Host encodes it as Ogg Opus.
  sendVoiceMessage: (communityId: string, channelId: string, pcmB64: string) =>
    hostInvoke<string>("send_voice_message", { communityId, channelId, pcmB64 }),
  expandCommunitySegment: (communityId: string) =>
    invoke<number>("expand_community_segment", { communityId }),
  /**
   * Architecture §10.7 + §20.6 — read the community-wide policy
   * (rules text + raid-protection thresholds). Falls back to the §20.6
   * defaults (20 joins per 600 s) when no `CommunityPolicy` entry has
   * been merged yet.
   */
  getCommunityPolicy: (communityId: string) =>
    hostInvoke<CommunityPolicy>("get_community_policy", { communityId }),
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
    hostInvoke<void>("set_community_policy", {
      communityId,
      policyText,
      maxJoinsPerInterval,
      joinIntervalSeconds,
    }),
  getChannelMessages: (channelId: string, limit: number) =>
    hostInvoke<Message[]>("get_channel_messages", { channelId, limit }),
  getOlderChannelMessages: (communityId: string, channelId: string, beforeTimestamp: number, limit: number) =>
    hostInvoke<Message[]>("get_older_channel_messages", { communityId, channelId, beforeTimestamp, limit }),
  createPoll: (
    communityId: string,
    channelId: string,
    messageId: string,
    question: string,
    answers: string[],
    multiSelect: boolean,
    durationSeconds?: number,
  ) => hostInvoke<string>("create_poll", {
    communityId,
    channelId,
    messageId,
    question,
    answers,
    multiSelect,
    durationSeconds: durationSeconds ?? null,
  }),
  votePoll: (communityId: string, channelId: string, pollId: string, selectedAnswers: number[]) =>
    hostInvoke<unknown>("vote_poll", { communityId, channelId, pollId, selectedAnswers }).then(() => undefined),
  closePoll: (communityId: string, channelId: string, pollId: string) =>
    hostInvoke<unknown>("close_poll", { communityId, channelId, pollId }).then(() => undefined),
  getPollResults: (communityId: string, channelId: string, pollId: string) =>
    hostInvoke<number[]>("get_poll_results", { communityId, channelId, pollId }),
  /// `kind` is the picked file's MIME type; the Host decodes it as that
  /// kind only and re-encodes it without metadata (`rekindle_types::ImageKind`).
  uploadEmoji: (communityId: string, name: string, bytes: number[], kind: ImageKind) =>
    hostInvoke<string>("upload_emoji", { communityId, name, bytes, kind }),
  uploadSticker: (
    communityId: string,
    name: string,
    bytes: number[],
    kind: ImageKind,
    tags?: string[],
  ) =>
    hostInvoke<string>("upload_sticker", {
      communityId,
      name,
      bytes,
      kind,
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
    hostInvoke<unknown>("delete_emoji", { communityId, expressionId }).then(() => undefined),
  listExpressions: (communityId: string) =>
    hostInvoke<ExpressionInfo[]>("list_expressions", { communityId }),
  listAutoModRules: (communityId: string) =>
    hostInvoke<AutoModRuleInfo[]>("list_automod_rules", { communityId }),
  setAutoModRule: (
    communityId: string,
    ruleId: string | null,
    name: string,
    enabled: boolean,
    keywords: string[],
    regexPatterns: string[],
    action: "blockLocally" | "blurContent" | "alertModerators",
  ) => hostInvoke<string>("set_automod_rule", {
    communityId,
    ruleId,
    name,
    enabled,
    keywords,
    regexPatterns,
    action,
  }),
  deleteAutoModRule: (communityId: string, ruleId: string) =>
    hostInvoke<unknown>("delete_automod_rule", { communityId, ruleId }).then(() => undefined),
  removeCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("remove_community_member", { communityId, pseudonymKey }).then(() => undefined),
  /** Enrol this device's fresh key in a community it may not write to
   *  (`CommunityDeviceEnrol`; the Host's TrustChanging review, `EnrolDevice`). */
  communityDeviceEnrol: (communityId: string) => hostInvoke<null>("community_device_enrol", { communityId }),
  leaveCommunity: (communityId: string) =>
    hostInvoke<unknown>("leave_community", { communityId }).then(() => undefined),
  deleteChannel: (communityId: string, channelId: string) =>
    hostInvoke<unknown>("delete_channel", { communityId, channelId }).then(() => undefined),
  renameChannel: (communityId: string, channelId: string, newName: string) =>
    hostInvoke<void>("rename_channel", { communityId, channelId, newName }),
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
    hostInvoke<unknown>("update_community_info", {
      communityId,
      name,
      description,
      iconHash,
      bannerHash,
    }),
  banMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("ban_member", { communityId, pseudonymKey }).then(() => undefined),
  unbanMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("unban_member", { communityId, pseudonymKey }).then(() => undefined),
  blockCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("block_community_member", { communityId, pseudonymKey }).then(() => undefined),
  unblockCommunityMember: (communityId: string, pseudonymKey: string) =>
    hostInvoke<unknown>("unblock_community_member", { communityId, pseudonymKey }).then(() => undefined),
  listCommunityBlocks: (communityId: string) =>
    hostInvoke<string[]>("list_community_blocks", { communityId }),
  getBanList: (communityId: string) =>
    hostInvoke<CommunityBanView[]>("get_ban_list", { communityId }),
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
  ) =>
    hostInvoke<unknown>("set_community_default_notification_level", { communityId, level }).then(
      () => undefined,
    ),
  /** Read the current community default notification level (or `null`). */
  getCommunityDefaultNotificationLevel: (communityId: string) =>
    hostInvoke<"all" | "mentions" | "nothing" | null>(
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
