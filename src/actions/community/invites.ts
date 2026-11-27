import { commands } from "../../ipc/commands";
import { setCommunityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import type { InviteCreated, InviteDto } from "../../ipc/commands/dto";

export async function handleCreateCommunityInvite(
  communityId: string,
  maxUses: number,
  expiresInSeconds?: number
): Promise<InviteCreated | null> {
  try {
    const result = await commands.createCommunityInvite(communityId, maxUses, expiresInSeconds);
    // The Host's list is the one source of each invite's fields, its
    // fingerprint among them (plan E7.15): never built here.
    await handleListCommunityInvites(communityId);
    return result;
  } catch (err) {
    console.error("[Community] Failed to create invite:", err);
    addToast("Failed to create invite", "error");
    return null;
  }
}

export async function handleRevokeCommunityInvite(
  communityId: string,
  inviteId: string
): Promise<boolean> {
  try {
    await commands.revokeCommunityInvite(communityId, inviteId);
    // Optimistic store removal
    setCommunityState("communityInvites", communityId, (prev) =>
      (prev ?? []).filter((inv) => inv.inviteId !== inviteId)
    );
    return true;
  } catch (err) {
    console.error("[Community] Failed to revoke invite:", err);
    addToast("Failed to revoke invite", "error");
    return false;
  }
}

export async function handleListCommunityInvites(communityId: string): Promise<InviteDto[]> {
  try {
    const invites = await commands.listCommunityInvites(communityId);
    setCommunityState("communityInvites", communityId, invites);
    return invites;
  } catch (err) {
    console.error("[Community] Failed to list invites:", err);
    addToast("Failed to load invites", "error");
    return [];
  }
}
