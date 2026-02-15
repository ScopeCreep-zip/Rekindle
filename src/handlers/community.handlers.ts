import { commands } from "../ipc/commands";
import { setCommunityState, communityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { addToast } from "../stores/toast.store";
import type { Message } from "../stores/chat.store";

export async function handleCreateCommunity(name: string): Promise<void> {
  try {
    const id = await commands.createCommunity(name);
    // Fetch full community details from backend (includes pseudonym, MEK gen, channels, roles)
    const details = await commands.getCommunityDetails();
    const created = details.find((c) => c.id === id);
    if (created) {
      setCommunityState("communities", id, {
        id: created.id,
        name: created.name,
        description: created.description ?? null,
        channels: created.channels.map((ch) => ({
          id: ch.id,
          name: ch.name,
          type: ch.channelType as "text" | "voice",
          unreadCount: ch.unreadCount,
        })),
        members: [],
        roles: created.roles ?? [],
        myRoleIds: created.myRoleIds ?? [0, 1],
        myPseudonymKey: created.myPseudonymKey ?? null,
        mekGeneration: created.mekGeneration ?? 0,
        isHosted: created.isHosted ?? true,
      });
    } else {
      setCommunityState("communities", id, {
        id,
        name,
        description: null,
        channels: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        isHosted: true,
      });
    }
  } catch (e) {
    console.error("Failed to create community:", e);
    addToast("Failed to create community", "error");
  }
}

export async function handleJoinCommunity(
  communityId: string,
  name: string,
): Promise<void> {
  try {
    await commands.joinCommunity(communityId);
    // Re-fetch community details to get channels, pseudonym key, MEK generation, roles
    const details = await commands.getCommunityDetails();
    const joined = details.find((c) => c.id === communityId);
    if (joined) {
      setCommunityState("communities", communityId, {
        id: joined.id,
        name: joined.name,
        description: joined.description ?? null,
        channels: joined.channels.map((ch) => ({
          id: ch.id,
          name: ch.name,
          type: ch.channelType as "text" | "voice",
          unreadCount: ch.unreadCount,
        })),
        members: [],
        roles: joined.roles ?? [],
        myRoleIds: joined.myRoleIds ?? [0, 1],
        myPseudonymKey: joined.myPseudonymKey ?? null,
        mekGeneration: joined.mekGeneration ?? 0,
        isHosted: joined.isHosted ?? false,
      });
    } else {
      setCommunityState("communities", communityId, {
        id: communityId,
        name,
        description: null,
        channels: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        isHosted: false,
      });
    }
  } catch (e) {
    console.error("Failed to join community:", e);
    addToast("Failed to join community", "error");
  }
}

export async function handleCreateChannel(
  communityId: string,
  name: string,
  channelType: "text" | "voice",
): Promise<void> {
  try {
    const channelId = await commands.createChannel(
      communityId,
      name,
      channelType,
    );
    setCommunityState("communities", communityId, "channels", (chs) => [
      ...chs,
      { id: channelId, name, type: channelType, unreadCount: 0 },
    ]);
  } catch (e) {
    console.error("Failed to create channel:", e);
    addToast("Failed to create channel", "error");
  }
}

export async function handleSendChannelMessage(
  channelId: string,
  body: string,
): Promise<void> {
  if (!body.trim()) return;
  const trimmed = body.trim();

  const tempId = Date.now();

  // Optimistic insert with "sending" status — use pseudonym key as senderId (matches what Rust emits)
  const community = communityState.communities[communityState.activeCommunity ?? ""];
  const message: Message = {
    id: tempId,
    senderId: community?.myPseudonymKey ?? authState.publicKey ?? "",
    body: trimmed,
    timestamp: Date.now(),
    isOwn: true,
    status: "sending",
  };

  const existing = communityState.channelMessages[channelId];
  if (existing) {
    setCommunityState("channelMessages", channelId, (msgs) => [
      ...msgs,
      message,
    ]);
  } else {
    setCommunityState("channelMessages", channelId, [message]);
  }

  try {
    await commands.sendChannelMessage(channelId, trimmed);
    // Update status to sent
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "sent" as const } : m)),
    );
  } catch (e) {
    console.error("Failed to send channel message:", e);
    addToast("Failed to send message", "error");
    // Update status to failed
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "failed" as const } : m)),
    );
  }
}

export async function handleRetryChannelMessage(
  channelId: string,
  messageId: number,
): Promise<void> {
  const messages = communityState.channelMessages[channelId];
  if (!messages) return;
  const message = messages.find((m) => m.id === messageId);
  if (!message || message.status !== "failed") return;

  setCommunityState("channelMessages", channelId, (msgs) =>
    msgs.map((m) => (m.id === messageId ? { ...m, status: "sending" as const } : m)),
  );

  try {
    await commands.sendChannelMessage(channelId, message.body);
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status: "sent" as const } : m)),
    );
  } catch {
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status: "failed" as const } : m)),
    );
  }
}

export async function handleLoadChannelMessages(
  channelId: string,
  limit: number,
): Promise<void> {
  try {
    const messages = await commands.getChannelMessages(channelId, limit);
    const mapped: Message[] = messages.map((m) => ({
      id: m.id,
      senderId: m.senderId,
      body: m.body,
      timestamp: m.timestamp,
      isOwn: m.isOwn,
    }));
    const existing = communityState.channelMessages[channelId];
    if (mapped.length > 0 || !existing || existing.length === 0) {
      setCommunityState("channelMessages", channelId, mapped);
    }
  } catch (e) {
    console.error("Failed to load channel messages:", e);
    addToast("Failed to load messages", "error");
  }
}

export function handleSelectCommunity(communityId: string): void {
  setCommunityState("activeCommunity", communityId);
  // Fetch members for the selected community
  commands.getCommunityMembers(communityId).then((members) => {
    setCommunityState("communities", communityId, "members", members.map((m) => ({
      pseudonymKey: m.pseudonymKey,
      displayName: m.displayName,
      roleIds: m.roleIds,
      displayRole: m.displayRole,
      status: m.status,
      timeoutUntil: m.timeoutUntil ?? null,
    })));
  }).catch((e) => {
    console.error("Failed to load community members:", e);
    addToast("Failed to load members", "error");
  });
  // Refresh community details to ensure myPseudonymKey, roles, and mekGeneration are current
  commands.getCommunityDetails().then((details) => {
    const detail = details.find((c) => c.id === communityId);
    if (detail) {
      setCommunityState("communities", communityId, "myPseudonymKey", detail.myPseudonymKey ?? null);
      setCommunityState("communities", communityId, "mekGeneration", detail.mekGeneration ?? 0);
      setCommunityState("communities", communityId, "isHosted", detail.isHosted ?? false);
      setCommunityState("communities", communityId, "myRoleIds", detail.myRoleIds ?? [0, 1]);
      setCommunityState("communities", communityId, "roles", detail.roles ?? []);
      setCommunityState("communities", communityId, "description", detail.description ?? null);
    }
  }).catch((e) => {
    console.error("Failed to refresh community details:", e);
    addToast("Failed to refresh community", "error");
  });
}

export function handleSelectChannel(channelId: string): void {
  setCommunityState("activeChannel", channelId);
}

export async function handleLeaveCommunity(communityId: string): Promise<void> {
  try {
    await commands.leaveCommunity(communityId);
    setCommunityState("communities", (prev) => {
      const next = { ...prev };
      delete next[communityId];
      return next;
    });
    // If we were viewing this community, clear the selection
    if (communityState.activeCommunity === communityId) {
      setCommunityState("activeCommunity", null);
      setCommunityState("activeChannel", null);
    }
  } catch (e) {
    console.error("Failed to leave community:", e);
    addToast("Failed to leave community", "error");
  }
}

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

export async function handleDeleteChannel(
  communityId: string,
  channelId: string,
): Promise<void> {
  try {
    await commands.deleteChannel(communityId, channelId);
    setCommunityState("communities", communityId, "channels", (chs) =>
      chs.filter((ch) => ch.id !== channelId),
    );
    // If the deleted channel was selected, clear selection
    if (communityState.activeChannel === channelId) {
      setCommunityState("activeChannel", null);
    }
  } catch (e) {
    console.error("Failed to delete channel:", e);
    addToast("Failed to delete channel", "error");
  }
}

export async function handleRenameChannel(
  communityId: string,
  channelId: string,
  newName: string,
): Promise<void> {
  try {
    await commands.renameChannel(communityId, channelId, newName);
    setCommunityState("communities", communityId, "channels",
      (ch) => ch.id === channelId,
      "name",
      newName,
    );
  } catch (e) {
    console.error("Failed to rename channel:", e);
    addToast("Failed to rename channel", "error");
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

// --- Role management handlers ---

export async function handleAssignRole(
  communityId: string,
  pseudonymKey: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.assignRole(communityId, pseudonymKey, roleId);
    // Update local state — add roleId to member
    setCommunityState("communities", communityId, "members",
      (m) => m.pseudonymKey === pseudonymKey,
      "roleIds",
      (ids) => [...ids, roleId],
    );
  } catch (e) {
    console.error("Failed to assign role:", e);
    addToast("Failed to assign role", "error");
  }
}

export async function handleUnassignRole(
  communityId: string,
  pseudonymKey: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.unassignRole(communityId, pseudonymKey, roleId);
    // Update local state — remove roleId from member
    setCommunityState("communities", communityId, "members",
      (m) => m.pseudonymKey === pseudonymKey,
      "roleIds",
      (ids) => ids.filter((id) => id !== roleId),
    );
  } catch (e) {
    console.error("Failed to unassign role:", e);
    addToast("Failed to unassign role", "error");
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

export async function handleCreateRole(
  communityId: string,
  name: string,
  color: number,
  permissions: number,
  hoist: boolean,
  mentionable: boolean,
): Promise<number | null> {
  try {
    const roleId = await commands.createRole(communityId, name, color, permissions, hoist, mentionable);
    // Optimistic update — add the new role to the store
    const community = communityState.communities[communityId];
    if (community) {
      const newRole = { id: roleId, name, color, permissions, position: 0, hoist, mentionable };
      setCommunityState("communities", communityId, "roles", [...community.roles, newRole]);
    }
    return roleId;
  } catch (e) {
    console.error("Failed to create role:", e);
    addToast("Failed to create role", "error");
    return null;
  }
}

export async function handleEditRole(
  communityId: string,
  roleId: number,
  name: string | null,
  color: number | null,
  permissions: number | null,
  position: number | null,
  hoist: boolean | null,
  mentionable: boolean | null,
): Promise<void> {
  try {
    await commands.editRole(communityId, roleId, name, color, permissions, position, hoist, mentionable);
    // Optimistic update — patch the role in the store
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.roles.findIndex((r) => r.id === roleId);
      if (idx >= 0) {
        const updated = { ...community.roles[idx] };
        if (name !== null) updated.name = name;
        if (color !== null) updated.color = color;
        if (permissions !== null) updated.permissions = permissions;
        if (position !== null) updated.position = position;
        if (hoist !== null) updated.hoist = hoist;
        if (mentionable !== null) updated.mentionable = mentionable;
        setCommunityState("communities", communityId, "roles", idx, updated);
      }
    }
  } catch (e) {
    console.error("Failed to edit role:", e);
    addToast("Failed to edit role", "error");
  }
}

export async function handleDeleteRole(
  communityId: string,
  roleId: number,
): Promise<void> {
  try {
    await commands.deleteRole(communityId, roleId);
    // Optimistic update — remove role from store and scrub from members
    const community = communityState.communities[communityId];
    if (community) {
      setCommunityState("communities", communityId, "roles",
        community.roles.filter((r) => r.id !== roleId),
      );
      // Scrub the deleted roleId from all members
      community.members.forEach((member, idx) => {
        if (member.roleIds.includes(roleId)) {
          setCommunityState("communities", communityId, "members", idx, "roleIds",
            member.roleIds.filter((id) => id !== roleId),
          );
        }
      });
      // Scrub from myRoleIds
      if (community.myRoleIds.includes(roleId)) {
        setCommunityState("communities", communityId, "myRoleIds",
          community.myRoleIds.filter((id) => id !== roleId),
        );
      }
    }
  } catch (e) {
    console.error("Failed to delete role:", e);
    addToast("Failed to delete role", "error");
  }
}
