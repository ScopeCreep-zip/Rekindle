import type { FriendInfo, Message as IpcMessage } from "../ipc/commands";
import type { DmMessage } from "../ipc/commands/dto_dm";
import type { DmHistoryGap } from "../ipc/channels/chat_events";
import type { ExpressionInfo } from "../ipc/commands";
import type { Friend } from "../stores/friends.store";
import type { GameInfo } from "../ipc/commands/dto";
import type { Message } from "../stores/chat.store";
import type { AutoModRule, Community, Channel, Expression, Member, MemberLocation } from "../stores/community.store";
import type { MemberGroup, TargetModeration } from "../ipc/commands/dto";
import { truncateKey } from "./formatting";

/** Convert a single backend FriendInfo to a frontend Friend. */
export function transformFriend(f: FriendInfo): Friend {
  return {
    publicKey: f.publicKey,
    displayName: f.displayName,
    nickname: f.nickname,
    status: (f.status as Friend["status"]) ?? "offline",
    statusMessage: f.statusMessage ?? null,
    gameInfo: f.gameInfo
      ? { gameName: f.gameInfo.gameName, gameId: f.gameInfo.gameId, startedAt: null, serverAddress: f.gameInfo.serverAddress ?? null, iconKey: f.gameInfo.iconKey }
      : null,
    group: f.groupLabel,
    groupId: f.groupId,
    unreadCount: 0,
    lastSeenAt: f.lastSeenAt ?? null,
    voiceChannel: null,
    friendshipState: (f.friendshipState as Friend["friendshipState"]) ?? "accepted",
    handshakeError: f.handshakeError ?? null,
    handshakeRetrying: f.handshakeRetrying,
    unreachable: false,
    identityPinMismatch: f.identityPinMismatch,
  };
}

/** Convert an array of FriendInfo into a keyed Record for the store. */
export function transformFriendMap(friends: FriendInfo[]): Record<string, Friend> {
  const map: Record<string, Friend> = {};
  for (const f of friends) {
    map[f.publicKey] = transformFriend(f);
  }
  return map;
}

/** Backend Message DTO → store Message: our own message's `status` comes
 *  from its stored `sendState`; optimistic sends set it themselves. */
export function transformMessage(m: IpcMessage): Message {
  const decryptionFailed = m.decryptionFailed ?? m.body === "(decryption failed)";
  return {
    id: m.id, senderId: m.senderId, body: decryptionFailed ? "" : m.body, timestamp: m.timestamp,
    isOwn: m.isOwn, serverMessageId: m.serverMessageId,
    decryptionFailed, automodBlurred: m.automodBlurred ?? false,
    reactions: m.reactions, pinned: m.pinned, poll: m.poll,
    forwardedFromAuthor: m.forwardedFromAuthor ?? null,
    attachment: m.attachment
      ? {
          attachmentId: m.attachment.attachmentId,
          filename: m.attachment.filename,
          mimeType: m.attachment.mimeType,
          totalSize: m.attachment.totalSize,
          chunkCount: m.attachment.chunkCount,
          downloaded: m.attachment.downloaded,
        }
      : undefined,
    flags: m.flags ?? 0,
    status: m.isOwn ? ownStatus(m.sendState) : undefined,
    failureReason: m.isOwn ? m.failureReason : undefined,
    editedAt: m.editedAt,
    removedBy: m.removedBy,
    historyGapThrough: m.historyGapThrough ?? undefined,
  };
}

/** Our own message's status from its stored delivery state: still queued,
 *  failed, or out (sent, delivered, read). */
function ownStatus(state: IpcMessage["sendState"]): Message["status"] {
  switch (state) {
    case "awaiting_key":
    case "queued":
    case "failed":
      return state;
    case "sent":
    case "delivered":
    case "read":
      return "sent";
  }
}

/** A direct message: its client id is the row's `serverMessageId`, which
 *  `DmSendStatus` and a resend name it by. */
export function transformDmMessage(m: DmMessage): Message {
  return {
    ...transformMessage(m),
    serverMessageId: m.messageId ?? undefined,
    historyGapThrough: m.historyGapThrough ?? undefined,
    expireSeconds: m.expireSeconds,
    expireAt: m.expireAt ?? undefined,
    timerNotice:
      m.timerNoticeSeconds === null
        ? undefined
        : { seconds: m.timerNoticeSeconds, setBy: m.senderId },
  };
}

/** A live `dmHistoryGap` event → the marker row the backend stored, as its
 *  history shows it (an empty, read row from the peer). The event carries
 *  no row id, so the live copy takes a local negative one; a reload reads
 *  the stored row. */
export function transformHistoryGap(gap: DmHistoryGap): Message {
  return {
    id: -gap.throughSeq,
    senderId: gap.peerKey,
    body: "",
    timestamp: Date.now(),
    isOwn: false,
    historyGapThrough: gap.throughSeq,
  };
}

/** Convenience: transform an array of messages. */
export function transformMessages(msgs: IpcMessage[]): Message[] {
  return msgs.map(transformMessage);
}

/** Backend channel DTO → store Channel (handles the channelType→type rename+cast). */
export function transformChannel(ch: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; slowmodeSeconds?: number; nsfw?: boolean; messageRecordKey?: string; mekGeneration?: number; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null }): Channel {
  return { id: ch.id, name: ch.name, type: ch.channelType as Channel["type"], unreadCount: 0, categoryId: ch.categoryId, topic: ch.topic ?? "", forumTags: ch.forumTags ?? undefined, stageSpeakers: ch.stageSpeakers ?? undefined, stageModerator: ch.stageModerator ?? null, slowmodeSeconds: ch.slowmodeSeconds, nsfw: ch.nsfw, messageRecordKey: ch.messageRecordKey, mekGeneration: ch.mekGeneration, notificationLevel: ch.notificationLevel ?? "all", notificationSoundRef: ch.notificationSoundRef ?? null };
}

export function transformExpression(expression: ExpressionInfo): Expression {
  return {
    id: expression.expressionId,
    name: expression.name,
    kind: expression.kind,
    contentHash: expression.contentHash,
    inlineDataUrl: expression.inlineDataUrl ?? null,
    mediaType: expression.mediaType ?? null,
    animated: expression.animated,
    tags: expression.tags ?? [],
    soundMeta: expression.soundMeta,
    creatorPseudonym: expression.creatorPseudonym,
    createdAt: expression.createdAt,
    availableToPeers: expression.availableToPeers,
  };
}

export function transformAutoModRule(rule: {
  ruleId: string;
  name: string;
  enabled: boolean;
  keywords: string[];
  regexPatterns: string[];
  action: "blockLocally" | "blurContent" | "alertModerators";
  lamport: number;
}): AutoModRule {
  return {
    ruleId: rule.ruleId,
    name: rule.name,
    enabled: rule.enabled,
    keywords: rule.keywords ?? [],
    regexPatterns: rule.regexPatterns ?? [],
    action: rule.action,
    lamport: rule.lamport,
  };
}

/** Backend community detail DTO → store Community. */
export function transformCommunityDetail(c: {
  id: string; name: string; description: string | null;
  iconHash?: string | null;
  bannerHash?: string | null;
  channels: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; slowmodeSeconds?: number; nsfw?: boolean; messageRecordKey?: string; mekGeneration?: number; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null }[];
  categories?: { id: string; name: string; sortOrder: number }[];
  roles?: { id: string; name: string; color: number; tagColours: { dark: number; light: number }; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean }[];
  myRoleIds?: string[]; myPseudonymKey?: string | null; mekGeneration?: number;
  myPermissions: string; myChannelPermissions: Record<string, string>;
  memberRegistryKey?: string; anchor?: string | null;
  onboardingComplete?: boolean;
  myBio?: string | null; myPronouns?: string | null;
  myThemeColor?: number | null; myBadges?: string[];
  removed?: {
    banned: boolean;
    by: string | null;
    byName: string | null;
    reason?: string | null;
  } | null;
}): Community {
  return {
    id: c.id, name: c.name, description: c.description ?? null,
    iconHash: c.iconHash ?? null,
    bannerHash: c.bannerHash ?? null,
    iconDataUrl: null,
    bannerDataUrl: null,
    channels: c.channels.map(transformChannel),
    categories: c.categories ?? [],
    members: [], roles: c.roles ?? [],
    myRoleIds: c.myRoleIds ?? [], myPseudonymKey: c.myPseudonymKey ?? null,
    myPermissions: c.myPermissions, myChannelPermissions: c.myChannelPermissions,
    mekGeneration: c.mekGeneration ?? 0,
    events: [],
    memberRegistryKey: c.memberRegistryKey, anchor: c.anchor ?? null,
    onboardingComplete: c.onboardingComplete ?? true,
    expressions: [],
    automodRules: [],
    myBio: c.myBio ?? null,
    myPronouns: c.myPronouns ?? null,
    myThemeColor: c.myThemeColor ?? null,
    myBadges: c.myBadges ?? [],
    removed: c.removed ?? null,
  };
}

/** Build a Record<id, Community> from an array of backend details. */
export function transformCommunityMap(
  details: Parameters<typeof transformCommunityDetail>[0][],
): Record<string, Community> {
  const map: Record<string, Community> = {};
  for (const c of details) { map[c.id] = transformCommunityDetail(c); }
  return map;
}

/** Backend member DTO → store Member. */
export function transformMember(m: {
  pseudonymKey: string; displayName: string; roleIds: string[];
  displayRole: string; status: string; timeoutUntil: number | null;
  gameInfo?: GameInfo | null;
  bio?: string | null; pronouns?: string | null;
  themeColor?: number | null; badges?: string[];
  location?: MemberLocation | null;
  lastActive?: number;
  group: MemberGroup;
  canModerateTarget: TargetModeration;
  outranks: boolean;
}): Member {
  return {
    // A member whose card is not read yet shows its short pseudonym, as
    // Matrix shows the user id without a displayname (plan E3.2 F9-2).
    pseudonymKey: m.pseudonymKey, displayName: m.displayName || truncateKey(m.pseudonymKey),
    roleIds: m.roleIds,
    displayRole: m.displayRole, status: m.status, timeoutUntil: m.timeoutUntil ?? null,
    gameInfo: m.gameInfo ?? null,
    bio: m.bio ?? null,
    pronouns: m.pronouns ?? null,
    themeColor: m.themeColor ?? null,
    badges: m.badges ?? [],
    location: m.location ?? null,
    lastActive: m.lastActive ?? 0,
    group: m.group,
    canModerateTarget: m.canModerateTarget,
    outranks: m.outranks,
  };
}

/** Minimal friendAdded event data → default Friend for store insertion. */
export function transformNewFriend(
  publicKey: string, displayName: string, friendshipState: string, groupLabel: string,
): Friend {
  return {
    publicKey, displayName, nickname: null, status: "offline" as const,
    statusMessage: null, gameInfo: null, group: groupLabel,
    // A friend is added in no group (the Host's `Added`).
    groupId: null, unreadCount: 0,
    lastSeenAt: null, voiceChannel: null,
    friendshipState: (friendshipState === "accepted" ? "accepted" : "pendingOut") as Friend["friendshipState"],
    handshakeError: null,
    handshakeRetrying: false,
    unreachable: false,
    identityPinMismatch: false,
  };
}


/** Presence event game data → store GameInfo. */
export function transformGameInfo(g: { gameName: string; gameId?: number | null; elapsedSeconds?: number | null; serverAddress?: string | null; iconKey: string | null }): GameInfo {
  return { gameName: g.gameName, gameId: g.gameId ?? null, startedAt: g.elapsedSeconds ?? null, serverAddress: g.serverAddress ?? null, iconKey: g.iconKey };
}
