import { hostInvoke, invoke } from "../invoke";
import type { VoiceMessageAudioView } from "./community";
import type { ImageKind } from "./system";
import type {
  AutoModRuleInfo,
  CommunityPolicy,
  ExpressionInfo,
  Message,
  SendChannelMessageResult,
} from "./types";

/** A community's channels and what is posted in them: messages,
 *  attachments, voice messages, polls, expressions and automod. */
export const communityContentCommands = {
  createChannel: (
    communityId: string,
    name: string,
    channelType: string,
    categoryId?: string,
    parentVoiceChannelId?: string
  ) =>
    hostInvoke<{ id: string }>("create_channel", {
      communityId,
      name,
      channelType,
      categoryId: categoryId ?? null,
      parentVoiceChannelId: parentVoiceChannelId ?? null,
    }).then((created) => created.id),
  sendChannelMessage: (channelId: string, body: string, replyToId?: string) =>
    hostInvoke<SendChannelMessageResult>("send_channel_message", {
      channelId,
      body,
      replyToId: replyToId ?? null,
    }),
  editChannelMessage: (channelId: string, messageId: string, newBody: string) =>
    hostInvoke<void>("edit_channel_message", { channelId, messageId, newBody }),
  deleteChannelMessage: (channelId: string, messageId: string) =>
    hostInvoke<void>("delete_channel_message", { channelId, messageId }),
  adminDeleteChannelMessage: (
    communityId: string,
    channelId: string,
    messageId: string,
    reason?: string
  ) =>
    hostInvoke<null>("admin_delete_channel_message", {
      communityId,
      channelId,
      messageId,
      reason: reason ?? null,
    }),
  bulkDeleteChannelMessages: (
    communityId: string,
    channelId: string,
    messageIds: string[],
    reason?: string
  ) =>
    hostInvoke<number>("bulk_delete_channel_messages", {
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
    destChannelId: string
  ) =>
    hostInvoke<{ status: string; messageId: string }>("forward_channel_message", {
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
    hostInvoke<unknown>("reveal_downloaded_attachment", {
      communityId,
      channelId,
      attachmentId,
    }).then(() => undefined),
  pinAttachment: (communityId: string, attachmentId: string, pinned: boolean) =>
    hostInvoke<unknown>("pin_attachment", { communityId, attachmentId, pinned }).then(
      () => undefined
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
    joinIntervalSeconds: number
  ) =>
    hostInvoke<void>("set_community_policy", {
      communityId,
      policyText,
      maxJoinsPerInterval,
      joinIntervalSeconds,
    }),
  getChannelMessages: (channelId: string, limit: number) =>
    hostInvoke<Message[]>("get_channel_messages", { channelId, limit }),
  getOlderChannelMessages: (
    communityId: string,
    channelId: string,
    beforeTimestamp: number,
    limit: number
  ) =>
    hostInvoke<Message[]>("get_older_channel_messages", {
      communityId,
      channelId,
      beforeTimestamp,
      limit,
    }),
  createPoll: (
    communityId: string,
    channelId: string,
    messageId: string,
    question: string,
    answers: string[],
    multiSelect: boolean,
    durationSeconds?: number
  ) =>
    hostInvoke<string>("create_poll", {
      communityId,
      channelId,
      messageId,
      question,
      answers,
      multiSelect,
      durationSeconds: durationSeconds ?? null,
    }),
  votePoll: (communityId: string, channelId: string, pollId: string, selectedAnswers: number[]) =>
    hostInvoke<unknown>("vote_poll", { communityId, channelId, pollId, selectedAnswers }).then(
      () => undefined
    ),
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
    tags?: string[]
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
    tags?: string[]
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
    action: "blockLocally" | "blurContent" | "alertModerators"
  ) =>
    hostInvoke<string>("set_automod_rule", {
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
};
