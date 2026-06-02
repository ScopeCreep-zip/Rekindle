import { commands } from "../../ipc/commands";
import { setCommunityState } from "../../stores/community.store";
import { authState } from "../../stores/auth.store";
import { addToast } from "../../stores/toast.store";
import type { InviteDto } from "../../stores/types";

export async function handleCreateCommunityInvite(
  communityId: string,
  maxUses?: number,
  expiresInSeconds?: number,
): Promise<{ code: string; governanceKey: string } | null> {
  try {
    const result = await commands.createCommunityInvite(communityId, maxUses, expiresInSeconds);
    // Optimistic store update — the raw code is only available to the creator
    const now = Math.floor(Date.now() / 1000);
    const newInvite: InviteDto = {
      codeHash: "pending", // Will be replaced by InviteCreated event
      createdBy: authState.publicKey ?? "",
      maxUses: maxUses ?? null,
      uses: 0,
      expiresAt: expiresInSeconds ? now + expiresInSeconds : null,
      createdAt: now,
    };
    setCommunityState("communityInvites", communityId, (prev) => [newInvite, ...(prev ?? [])]);
    return result;
  } catch (err) {
    console.error("[Community] Failed to create invite:", err);
    addToast("Failed to create invite", "error");
    return null;
  }
}

export async function handleRevokeCommunityInvite(
  communityId: string,
  codeHash: string,
): Promise<boolean> {
  try {
    await commands.revokeCommunityInvite(communityId, codeHash);
    // Optimistic store removal
    setCommunityState("communityInvites", communityId, (prev) =>
      (prev ?? []).filter((inv) => inv.codeHash !== codeHash),
    );
    return true;
  } catch (err) {
    console.error("[Community] Failed to revoke invite:", err);
    addToast("Failed to revoke invite", "error");
    return false;
  }
}

export async function handleListCommunityInvites(
  communityId: string,
): Promise<InviteDto[]> {
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
