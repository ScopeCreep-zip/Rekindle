import type { FriendInfo, Message as IpcMessage } from "../ipc/commands";
import type { ExpressionInfo } from "../ipc/commands";
import type { Friend } from "../stores/friends.store";
import type { GameInfo } from "../stores/types";
import type { Message } from "../stores/chat.store";
import type { AutoModRule, Community, Channel, Expression, Member } from "../stores/community.store";

/** Convert a single backend FriendInfo to a frontend Friend. */
export function transformFriend(f: FriendInfo): Friend {
  return {
    publicKey: f.publicKey,
    displayName: f.displayName,
    nickname: f.nickname,
    status: (f.status as Friend["status"]) ?? "offline",
    statusMessage: f.statusMessage ?? null,
    gameInfo: f.gameInfo
      ? { gameName: f.gameInfo.gameName, gameId: f.gameInfo.gameId, startedAt: null, serverAddress: f.gameInfo.serverAddress ?? null }
      : null,
    group: f.group ?? "Friends",
    unreadCount: f.unreadCount,
    lastSeenAt: f.lastSeenAt ?? null,
    voiceChannel: null,
    friendshipState: (f.friendshipState as Friend["friendshipState"]) ?? "accepted",
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

/** Backend Message DTO → store Message (no status field — callers add for optimistic sends). */
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
          localPath: m.attachment.localPath ?? null,
        }
      : undefined,
    flags: m.flags ?? 0,
  };
}

/** Convenience: transform an array of messages. */
export function transformMessages(msgs: IpcMessage[]): Message[] {
  return msgs.map(transformMessage);
}

/** Backend channel DTO → store Channel (handles the channelType→type rename+cast). */
export function transformChannel(ch: { id: string; name: string; channelType: string; unreadCount: number; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; slowmodeSeconds?: number; nsfw?: boolean; messageRecordKey?: string; mekGeneration?: number; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null }): Channel {
  return { id: ch.id, name: ch.name, type: ch.channelType as Channel["type"], unreadCount: ch.unreadCount, categoryId: ch.categoryId, topic: ch.topic ?? "", forumTags: ch.forumTags ?? undefined, stageSpeakers: ch.stageSpeakers ?? undefined, stageModerator: ch.stageModerator ?? null, slowmodeSeconds: ch.slowmodeSeconds, nsfw: ch.nsfw, messageRecordKey: ch.messageRecordKey, mekGeneration: ch.mekGeneration, notificationLevel: ch.notificationLevel ?? "all", notificationSoundRef: ch.notificationSoundRef ?? null };
}

export function transformExpression(expression: ExpressionInfo): Expression {
  const inlineDataUrl = expression.inlineDataBase64
    ? `data:${expression.mediaType ?? "image/png"};base64,${expression.inlineDataBase64}`
    : null;
  return {
    id: expression.expressionId,
    name: expression.name,
    kind: expression.kind,
    contentHash: expression.contentHash,
    inlineDataBase64: expression.inlineDataBase64 ?? null,
    inlineDataUrl,
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
  action: "block_locally" | "blur_content" | "alert_moderators";
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
  channels: { id: string; name: string; channelType: string; unreadCount: number; categoryId?: string; topic?: string; forumTags?: string[]; stageSpeakers?: string[]; stageModerator?: string | null; slowmodeSeconds?: number; nsfw?: boolean; messageRecordKey?: string; mekGeneration?: number; notificationLevel?: "all" | "mentions" | "nothing"; notificationSoundRef?: string | null }[];
  categories?: { id: string; name: string; sortOrder: number }[];
  roles?: { id: number; name: string; color: number; permissions: string; position: number; hoist: boolean; mentionable: boolean; selfAssignable?: boolean }[];
  myRoleIds?: number[]; myPseudonymKey?: string | null; mekGeneration?: number;
  memberRegistryKey?: string; governanceKey?: string | null;
  onboardingComplete?: boolean;
  myBio?: string | null; myPronouns?: string | null;
  myThemeColor?: number | null; myBadges?: string[];
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
    myRoleIds: c.myRoleIds ?? [0, 1], myPseudonymKey: c.myPseudonymKey ?? null,
    mekGeneration: c.mekGeneration ?? 0,
    events: [],
    memberRegistryKey: c.memberRegistryKey, governanceKey: c.governanceKey ?? null,
    onboardingComplete: c.onboardingComplete ?? true,
    expressions: [],
    automodRules: [],
    myBio: c.myBio ?? null,
    myPronouns: c.myPronouns ?? null,
    myThemeColor: c.myThemeColor ?? null,
    myBadges: c.myBadges ?? [],
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
  pseudonymKey: string; displayName: string; roleIds: number[];
  displayRole: string; status: string; timeoutUntil: number | null;
  gameInfo?: GameInfo | null;
  bio?: string | null; pronouns?: string | null;
  themeColor?: number | null; badges?: string[];
}): Member {
  return {
    pseudonymKey: m.pseudonymKey, displayName: m.displayName, roleIds: m.roleIds,
    displayRole: m.displayRole, status: m.status, timeoutUntil: m.timeoutUntil ?? null,
    gameInfo: m.gameInfo ?? null,
    bio: m.bio ?? null,
    pronouns: m.pronouns ?? null,
    themeColor: m.themeColor ?? null,
    badges: m.badges ?? [],
  };
}

/** Minimal friendAdded event data → default Friend for store insertion. */
export function transformNewFriend(publicKey: string, displayName: string, friendshipState: string): Friend {
  return {
    publicKey, displayName, nickname: null, status: "offline" as const,
    statusMessage: null, gameInfo: null, group: "Friends", unreadCount: 0,
    lastSeenAt: null, voiceChannel: null,
    friendshipState: (friendshipState === "accepted" ? "accepted" : "pendingOut") as Friend["friendshipState"],
  };
}


/** Presence event game data → store GameInfo. */
export function transformGameInfo(g: { gameName: string; gameId?: number | null; elapsedSeconds?: number | null; serverAddress?: string | null }): GameInfo {
  return { gameName: g.gameName, gameId: g.gameId ?? null, startedAt: g.elapsedSeconds ?? null, serverAddress: g.serverAddress ?? null };
}
