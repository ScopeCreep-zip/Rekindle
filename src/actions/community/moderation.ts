import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";

export async function handleRemoveCommunityMember(
  communityId: string,
  pseudonymKey: string,
): Promise<void> {
  try {
    await commands.removeCommunityMember(communityId, pseudonymKey);
    setCommunityState("communities", communityId, "members", (members) =>
      members.filter((m) => m.pseudonymKey !== pseudonymKey),
    );
  } catch (e) {
    console.error("Failed to remove community member:", e);
    addToast("Failed to remove member", "error");
  }
}

export async function handleUpdateCommunityInfo(
  communityId: string,
  name: string | null,
  description: string | null,
): Promise<void> {
  try {
    await commands.updateCommunityInfo(communityId, name, description);
    if (name !== null) {
      setCommunityState("communities", communityId, "name", name);
    }
    if (description !== null) {
      setCommunityState("communities", communityId, "description", description);
    }
  } catch (e) {
    console.error("Failed to update community info:", e);
    addToast("Failed to update community", "error");
  }
}

export async function handleBanMember(
  communityId: string,
  pseudonymKey: string,
): Promise<void> {
  try {
    await commands.banMember(communityId, pseudonymKey);
    setCommunityState("communities", communityId, "members", (members) =>
      members.filter((m) => m.pseudonymKey !== pseudonymKey),
    );
  } catch (e) {
    console.error("Failed to ban member:", e);
    addToast("Failed to ban member", "error");
  }
}

export async function handleUnbanMember(
  communityId: string,
  pseudonymKey: string,
): Promise<void> {
  try {
    await commands.unbanMember(communityId, pseudonymKey);
  } catch (e) {
    console.error("Failed to unban member:", e);
    addToast("Failed to unban member", "error");
  }
}

// Architecture §10 — moderator voice actions. Server-mute / server-
// deafen broadcast a `Control` envelope through the gossip mesh and
// require `MUTE_MEMBERS` / `DEAFEN_MEMBERS` respectively (validated
// at the backend boundary; the menu also gates UX so the option only
// surfaces when the local user has the perm and the target is in
// `voiceChannels[channelId].participants`).
export async function handleServerMuteMember(
  communityId: string,
  channelId: string,
  targetPseudonym: string,
  muted: boolean,
): Promise<void> {
  try {
    await commands.serverMuteMember(communityId, channelId, targetPseudonym, muted);
    addToast(muted ? "Member server-muted" : "Member un-muted", "success");
  } catch (e) {
    console.error("Failed to server-mute member:", e);
    addToast("Failed to update server-mute", "error");
  }
}

export async function handleServerDeafenMember(
  communityId: string,
  channelId: string,
  targetPseudonym: string,
  deafened: boolean,
): Promise<void> {
  try {
    await commands.serverDeafenMember(
      communityId,
      channelId,
      targetPseudonym,
      deafened,
    );
    addToast(deafened ? "Member server-deafened" : "Member un-deafened", "success");
  } catch (e) {
    console.error("Failed to server-deafen member:", e);
    addToast("Failed to update server-deafen", "error");
  }
}

export async function handleGetBanList(
  communityId: string,
): Promise<{ pseudonymKey: string; displayName: string; bannedAt: number }[]> {
  try {
    return await commands.getBanList(communityId);
  } catch (e) {
    console.error("Failed to get ban list:", e);
    addToast("Failed to load ban list", "error");
    return [];
  }
}

export async function handleRotateMek(
  communityId: string,
): Promise<void> {
  try {
    await commands.rotateMek(communityId);
  } catch (e) {
    console.error("Failed to rotate MEK:", e);
    addToast("Failed to rotate encryption key", "error");
  }
}

export async function handleGetAuditLog(
  communityId: string,
  beforeTimestamp?: number,
  limit: number = 50,
): Promise<{ action: string; actorPseudonym: string; target: string | null; details: string | null; timestamp: number }[]> {
  try {
    return await commands.getAuditLog(communityId, beforeTimestamp, limit);
  } catch (e) {
    console.error("Failed to get audit log:", e);
    return [];
  }
}

export async function handleTimeoutMember(
  communityId: string,
  pseudonymKey: string,
  durationSeconds: number,
  reason: string | null,
): Promise<void> {
  try {
    await commands.timeoutMember(communityId, pseudonymKey, durationSeconds, reason);
    // Optimistic update — compute timeout_until in seconds
    const timeoutUntil = Math.floor(Date.now() / 1000) + durationSeconds;
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "timeoutUntil", timeoutUntil);
      }
    }
  } catch (e) {
    console.error("Failed to timeout member:", e);
    addToast("Failed to timeout member", "error");
  }
}

export async function handleRemoveTimeout(
  communityId: string,
  pseudonymKey: string,
): Promise<void> {
  try {
    await commands.removeTimeout(communityId, pseudonymKey);
    // Optimistic update — clear timeout
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "timeoutUntil", null);
      }
    }
  } catch (e) {
    console.error("Failed to remove timeout:", e);
    addToast("Failed to remove timeout", "error");
  }
}
