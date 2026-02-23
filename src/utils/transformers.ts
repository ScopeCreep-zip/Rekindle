import type { FriendInfo, Message as IpcMessage } from "../ipc/commands";
import type { Friend, GameInfo } from "../stores/friends.store";
import type { Message } from "../stores/chat.store";
import type { Community, Channel, Member } from "../stores/community.store";

/** Convert a single backend FriendInfo to a frontend Friend. */
export function transformFriend(f: FriendInfo): Friend {
  return {
    publicKey: f.publicKey,
    displayName: f.displayName,
    nickname: f.nickname,
    status: (f.status as Friend["status"]) ?? "offline",
    statusMessage: f.statusMessage ?? null,
    gameInfo: f.gameInfo
      ? { gameName: f.gameInfo.gameName, gameId: f.gameInfo.gameId, startedAt: null }
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
  return { id: m.id, senderId: m.senderId, body: m.body, timestamp: m.timestamp, isOwn: m.isOwn };
}

/** Convenience: transform an array of messages. */
export function transformMessages(msgs: IpcMessage[]): Message[] {
  return msgs.map(transformMessage);
}

/** Backend channel DTO → store Channel (handles the channelType→type rename+cast). */
export function transformChannel(ch: { id: string; name: string; channelType: string; unreadCount: number }): Channel {
  return { id: ch.id, name: ch.name, type: ch.channelType as "text" | "voice", unreadCount: ch.unreadCount };
}

/** Backend community detail DTO → store Community. */
export function transformCommunityDetail(c: {
  id: string; name: string; description: string | null;
  channels: { id: string; name: string; channelType: string; unreadCount: number }[];
  roles?: { id: number; name: string; color: number; permissions: number; position: number; hoist: boolean; mentionable: boolean }[];
  myRoleIds?: number[]; myPseudonymKey?: string | null; mekGeneration?: number; isHosted?: boolean;
}): Community {
  return {
    id: c.id, name: c.name, description: c.description ?? null,
    channels: c.channels.map(transformChannel),
    members: [], roles: c.roles ?? [],
    myRoleIds: c.myRoleIds ?? [0, 1], myPseudonymKey: c.myPseudonymKey ?? null,
    mekGeneration: c.mekGeneration ?? 0, isHosted: c.isHosted ?? false,
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
}): Member {
  return {
    pseudonymKey: m.pseudonymKey, displayName: m.displayName, roleIds: m.roleIds,
    displayRole: m.displayRole, status: m.status, timeoutUntil: m.timeoutUntil ?? null,
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
export function transformGameInfo(g: { gameName: string; gameId?: number | null; elapsedSeconds?: number | null }): GameInfo {
  return { gameName: g.gameName, gameId: g.gameId ?? null, startedAt: g.elapsedSeconds ?? null };
}
