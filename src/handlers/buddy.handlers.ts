import { reconcile } from "solid-js/store";
import { commands } from "../ipc/commands";
import { setFriendsState, friendsState } from "../stores/friends.store";
import { authState } from "../stores/auth.store";
import type { OutgoingInvite } from "../stores/friends.store";
import { transformFriendMap } from "../utils/transformers";
import { errorMessage } from "../utils/error";

export function handleDoubleClickFriend(
  publicKey: string,
  displayName: string,
): void {
  commands.openChatWindow(publicKey, displayName);
}

export async function handleRemoveFriend(publicKey: string): Promise<void> {
  try {
    await commands.removeFriend(publicKey);
    // Backend emits FriendRemoved event which handles store update via reconcile.
    // No inline store mutation needed here.
  } catch (e) {
    console.error("Failed to remove friend:", e);
  }
}

/// B6 — explicit Signal session reset for a friend. The user must
/// confirm because resetting the session means: (1) the next encrypted
/// send to this peer will fail with "no session" until both sides
/// re-handshake, (2) old undecryptable messages stay undecryptable,
/// (3) the new session may be MitM'd if the peer's safety number isn't
/// verified out-of-band first. Returns null on success, or the error
/// string for the caller's toast.
export async function handleResetSignalSession(
  publicKey: string,
  displayName?: string,
): Promise<string | null> {
  const name = displayName ?? publicKey.slice(0, 12);
  const confirmed = window.confirm(
    `Reset secure session with ${name}?\n\n` +
      `This will delete the current Signal session. Both of you will need ` +
      `to re-handshake on the next message exchange. Old undecryptable ` +
      `messages stay lost.\n\n` +
      `Verify their safety number out-of-band before resuming sensitive ` +
      `conversations after the new session is established.`,
  );
  if (!confirmed) return null;
  try {
    await commands.resetSignalSession(publicKey);
    return null;
  } catch (e) {
    return errorMessage(e);
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
    return errorMessage(e);
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
    return errorMessage(e);
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
    return errorMessage(e);
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

export async function handleBlockUser(publicKey: string, displayName?: string): Promise<string | null> {
  try {
    await commands.blockUser(publicKey, displayName);
    // Remove from friends store
    const next = { ...friendsState.friends };
    delete next[publicKey];
    setFriendsState("friends", reconcile(next));
    // Remove from pending requests store
    setFriendsState("pendingRequests", (reqs) =>
      reqs.filter((r) => r.publicKey !== publicKey),
    );
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export async function handleUnblockUser(publicKey: string): Promise<string | null> {
  try {
    await commands.unblockUser(publicKey);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export async function handleGenerateInvite(): Promise<{ url: string; inviteId: string } | string> {
  try {
    const result = await commands.generateInvite();
    // Append to store so the invite list updates immediately
    const now = Date.now();
    const newInvite: OutgoingInvite = {
      inviteId: result.inviteId,
      url: result.url,
      createdAt: now,
      expiresAt: now + 48 * 60 * 60 * 1000, // 48h default
      status: "pending",
      acceptedBy: null,
    };
    setFriendsState("outgoingInvites", (prev) => [...prev, newInvite]);
    return result;
  } catch (e) {
    console.error("Failed to generate invite:", e);
    return errorMessage(e);
  }
}

export async function handleCancelInvite(inviteId: string): Promise<string | null> {
  try {
    await commands.cancelInvite(inviteId);
    setFriendsState("outgoingInvites", (prev) =>
      prev.filter((inv) => inv.inviteId !== inviteId),
    );
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export async function handleLoadOutgoingInvites(): Promise<void> {
  try {
    const invites = await commands.getOutgoingInvites();
    setFriendsState(
      "outgoingInvites",
      invites.map((inv) => ({
        inviteId: inv.inviteId,
        url: inv.url,
        createdAt: inv.createdAt,
        expiresAt: inv.expiresAt,
        status: inv.status,
        acceptedBy: inv.acceptedBy,
      })),
    );
  } catch (e) {
    console.error("Failed to load outgoing invites:", e);
  }
}

export async function handleAddFriendFromInvite(inviteString: string): Promise<string | null> {
  try {
    await commands.addFriendFromInvite(inviteString);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export async function handleCancelRequest(publicKey: string): Promise<string | null> {
  try {
    await commands.cancelRequest(publicKey);
    const next = { ...friendsState.friends };
    delete next[publicKey];
    setFriendsState("friends", reconcile(next));
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

export async function handleRefreshFriends(): Promise<void> {
  try {
    const friends = await commands.getFriends();
    setFriendsState("friends", reconcile(transformFriendMap(friends)));
  } catch (e) {
    console.error("Failed to refresh friends:", e);
  }
}
