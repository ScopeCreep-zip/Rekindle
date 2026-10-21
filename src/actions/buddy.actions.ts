import { reconcile } from "solid-js/store";
import { commands } from "../ipc/commands";
import { setFriendsState, friendsState } from "../stores/friends.store";
import { authState } from "../stores/auth.store";
import type { OutgoingInvite } from "../stores/friends.store";
import { transformFriendMap } from "../utils/transformers";
import { errorMessage } from "../utils/error";

export function handleDoubleClickFriend(publicKey: string): void {
  commands.openChatWindow(publicKey);
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

/// B6 — explicit Signal session reset for a friend. The Host's review
/// (TrustChanging, plan E2.1) is the confirmation: it names whose session
/// it is, and the friend compares safety numbers before accepting. Returns
/// null on success, or the error string for the caller's toast.
export async function handleResetSignalSession(publicKey: string): Promise<string | null> {
  try {
    await commands.resetSignalSession(publicKey);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/// Send a friend request to the identity a friend code names
/// (`rekindle://add/...`, plan E2.5 D17).
export async function handleAddFriendFromCode(
  code: string,
  message: string,
): Promise<string | null> {
  try {
    // The Host names them from its own rows (audit U-7). Not time-bounded here: the Host's review may be on screen, and the
    // Host bounds its own wait for our route (plan E2.1).
    await commands.addFriendFromCode(code, message);
    return null;
  } catch (e) {
    return errorMessage(e);
  }
}

/// B7 — retry the first contact whose secure session did not open. The
/// Host re-sends it and publishes HandshakeRetrying, then HandshakeFailed
/// again if it cannot; the row follows those events.
export async function handleRetryHandshake(publicKey: string): Promise<void> {
  try {
    await commands.retryHandshake(publicKey);
  } catch (e) {
    console.error("Retry of the secure session failed:", errorMessage(e));
  }
}

export async function handleAcceptRequest(publicKey: string): Promise<string | null> {
  try {
    // The Host names them from their request (audit U-7).
    await commands.acceptRequest(publicKey);
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
    const { groupId } = await commands.createFriendGroup(name);
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
    // The Host announces each relabelled friend (`groupChanged`).
    await commands.renameFriendGroup(groupId, name);
  } catch (e) {
    console.error("Failed to rename friend group:", e);
  }
}

export async function handleMoveFriendToGroup(
  publicKey: string,
  groupId: number | null,
): Promise<void> {
  try {
    // The Host announces the move (`groupChanged`).
    await commands.moveFriendToGroup(publicKey, groupId);
  } catch (e) {
    console.error("Failed to move friend to group:", e);
  }
}

export async function handleBlockUser(publicKey: string): Promise<string | null> {
  try {
    await commands.blockUser(publicKey);
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
    const newInvite: OutgoingInvite = {
      inviteId: result.inviteId,
      url: result.url,
      createdAt: result.createdAt,
      expiresAt: result.expiresAt,
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
    // Reviewed with the inviter's fingerprint first: not time-bounded.
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
