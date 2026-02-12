import { commands } from "../ipc/commands";
import { setFriendsState, friendsState } from "../stores/friends.store";
import { authState } from "../stores/auth.store";
import type { Friend } from "../stores/friends.store";

export function handleDoubleClickFriend(
  publicKey: string,
  displayName: string,
): void {
  commands.openChatWindow(publicKey, displayName);
}

export function handleContextMenuFriend(
  e: MouseEvent,
  publicKey: string,
): void {
  e.preventDefault();
  setFriendsState("contextMenu", { x: e.clientX, y: e.clientY, publicKey });
}

export function handleCloseContextMenu(): void {
  setFriendsState("contextMenu", null);
}

export async function handleRemoveFriend(publicKey: string): Promise<void> {
  try {
    await commands.removeFriend(publicKey);
    setFriendsState("friends", (prev) => {
      const next = { ...prev };
      delete next[publicKey];
      return next;
    });
  } catch (e) {
    console.error("Failed to remove friend:", e);
  }
}

export async function handleAddFriend(
  publicKey: string,
  message: string,
): Promise<string | null> {
  try {
    const displayName = publicKey.slice(0, 12) + "...";
    await commands.addFriend(publicKey, displayName, message);
    return null;
  } catch (e) {
    return String(e);
  }
}

export async function handleAcceptRequest(publicKey: string): Promise<string | null> {
  try {
    // Look up the display name from the pending request
    const pending = friendsState.pendingRequests.find((r) => r.publicKey === publicKey);
    const displayName = pending?.displayName ?? publicKey.slice(0, 12) + "...";
    await commands.acceptRequest(publicKey, displayName);
    setFriendsState("pendingRequests", (reqs) =>
      reqs.filter((r) => r.publicKey !== publicKey),
    );
    return null;
  } catch (e) {
    return String(e);
  }
}

export async function handleRejectRequest(publicKey: string): Promise<string | null> {
  try {
    await commands.rejectRequest(publicKey);
    setFriendsState("pendingRequests", (reqs) =>
      reqs.filter((r) => r.publicKey !== publicKey),
    );
    return null;
  } catch (e) {
    return String(e);
  }
}

export async function handleLoadPendingRequests(): Promise<void> {
  try {
    const requests = await commands.getPendingRequests();
    setFriendsState(
      "pendingRequests",
      requests.map((r) => ({
        publicKey: r.publicKey,
        displayName: r.displayName,
        message: r.message,
      })),
    );
  } catch (e) {
    console.error("Failed to load pending requests:", e);
  }
}

export function handleCopyPublicKey(): void {
  navigator.clipboard.writeText(authState.publicKey ?? "");
}

export function handleToggleAddFriend(): void {
  setFriendsState("showAddFriend", (v) => !v);
}

export async function handleCreateFriendGroup(name: string): Promise<number> {
  try {
    const groupId = await commands.createFriendGroup(name);
    return groupId;
  } catch (e) {
    console.error("Failed to create friend group:", e);
    return -1;
  }
}

export async function handleRenameFriendGroup(
  groupId: number,
  name: string,
): Promise<void> {
  try {
    await commands.renameFriendGroup(groupId, name);
    // Refresh friends to update group names in the store
    await handleRefreshFriends();
  } catch (e) {
    console.error("Failed to rename friend group:", e);
  }
}

export async function handleMoveFriendToGroup(
  publicKey: string,
  groupId: number | null,
): Promise<void> {
  try {
    await commands.moveFriendToGroup(publicKey, groupId);
    // Refresh friends to update the group assignment
    await handleRefreshFriends();
  } catch (e) {
    console.error("Failed to move friend to group:", e);
  }
}

export async function handleRefreshFriends(): Promise<void> {
  try {
    const friends = await commands.getFriends();
    const friendMap: Record<string, Friend> = {};
    for (const f of friends) {
      friendMap[f.publicKey] = {
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
      };
    }
    setFriendsState("friends", friendMap);
  } catch (e) {
    console.error("Failed to refresh friends:", e);
  }
}
