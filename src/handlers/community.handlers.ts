import type { UnlistenFn } from "@tauri-apps/api/event";
import { createStore } from "solid-js/store";
import { commands } from "../ipc/commands";
import { subscribeCommunityEvents } from "../ipc/channels";
import { setCommunityState, communityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import { addToast } from "../stores/toast.store";
import type { Message } from "../stores/chat.store";
import type { Channel, CommunityEvent as CommunityEventType } from "../stores/community.store";
import type { InviteDto } from "../stores/types";
import { transformCommunityDetail, transformMember, transformMessages } from "../utils/transformers";
import { truncateKey } from "../utils/formatting";

// Typing indicator state
export interface TypingUser {
  pseudonymKey: string;
  displayName: string;
}

const [typingUsersStore, setTypingUsers] = createStore<Record<string, TypingUser[]>>({});
const typingTimers: Record<string, number> = {};

export { typingUsersStore as typingUsers };

export async function handleCreateCommunity(name: string): Promise<void> {
  try {
    const id = await commands.createCommunity(name);
    // Fetch full community details from backend (includes pseudonym, MEK gen, channels, roles)
    const details = await commands.getCommunityDetails();
    const created = details.find((c) => c.id === id);
    if (created) {
      setCommunityState("communities", id, transformCommunityDetail(created));
    } else {
      setCommunityState("communities", id, {
        id,
        name,
        description: null,
        channels: [],
        categories: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        events: [],
      });
    }
    // Fetch members so the creator appears in the member list
    try {
      const members = await commands.getCommunityMembers(id);
      setCommunityState("communities", id, "members", members.map(transformMember));
    } catch (e) {
      console.error("Failed to load community members after creation:", e);
    }
  } catch (e) {
    console.error("Failed to create community:", e);
    addToast("Failed to create community", "error");
  }
}

export async function handleJoinCommunity(
  communityId: string,
  name: string,
  inviteCode?: string,
): Promise<void> {
  try {
    await commands.joinCommunity(communityId, inviteCode);
    // Re-fetch community details to get channels, pseudonym key, MEK generation, roles
    const details = await commands.getCommunityDetails();
    const joined = details.find((c) => c.id === communityId);
    if (joined) {
      setCommunityState("communities", communityId, transformCommunityDetail(joined));
    } else {
      setCommunityState("communities", communityId, {
        id: communityId,
        name,
        description: null,
        channels: [],
        categories: [],
        members: [],
        roles: [],
        myRoleIds: [0, 1],
        myPseudonymKey: null,
        mekGeneration: 0,
        events: [],
      });
    }
    // Fetch members for the newly joined community
    try {
      const members = await commands.getCommunityMembers(communityId);
      setCommunityState("communities", communityId, "members", members.map(transformMember));
    } catch (e) {
      console.error("Failed to load community members after join:", e);
    }

    // Auto-select the newly joined community so it appears immediately
    handleSelectCommunity(communityId);
    addToast("Joined community!", "success");
  } catch (e) {
    console.error("Failed to join community:", e);
    addToast("Failed to join community", "error");
  }
}

export async function handleCreateChannel(
  communityId: string,
  name: string,
  channelType: string,
  categoryId?: string,
): Promise<void> {
  try {
    const channelId = await commands.createChannel(
      communityId,
      name,
      channelType,
      categoryId,
    );
    setCommunityState("communities", communityId, "channels", (chs) => [
      ...chs,
      { id: channelId, name, type: channelType as Channel["type"], unreadCount: 0, categoryId },
    ]);
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to create channel";
    console.error("Failed to create channel:", e);
    addToast(msg, "error");
    throw e;
  }
}

export async function handleSendChannelMessage(
  channelId: string,
  body: string,
  replyToId?: string,
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
    replyToId,
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
    const result = await commands.sendChannelMessage(channelId, trimmed, replyToId);
    const status = result === "queued" ? ("queued" as const) : ("sent" as const);
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status } : m)),
    );
    if (result === "queued") {
      addToast("Message queued — will deliver when server is reachable", "info");
    }
  } catch (e) {
    console.error("Failed to send channel message:", e);
    addToast("Failed to send message", "error");
    // Update status to failed
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "failed" as const } : m)),
    );
  }
}

export async function handleEditChannelMessage(
  channelId: string,
  messageId: string,
  newBody: string,
): Promise<void> {
  try {
    await commands.editChannelMessage(channelId, messageId, newBody);
  } catch (e) {
    console.error("Failed to edit message:", e);
    addToast("Failed to edit message", "error");
  }
}

export async function handleDeleteChannelMessage(
  channelId: string,
  messageId: string,
): Promise<void> {
  try {
    await commands.deleteChannelMessage(channelId, messageId);
  } catch (e) {
    console.error("Failed to delete message:", e);
    addToast("Failed to delete message", "error");
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

export async function handleLoadOlderMessages(
  communityId: string,
  channelId: string,
  beforeTimestamp: number,
  limit: number = 50,
): Promise<boolean> {
  try {
    const messages = await commands.getOlderChannelMessages(communityId, channelId, beforeTimestamp, limit);
    if (messages.length === 0) return false;
    const mapped = transformMessages(messages);
    setCommunityState("channelMessages", channelId, (prev) => [...mapped, ...(prev ?? [])]);
    return messages.length >= limit;
  } catch (e) {
    console.error("Failed to load older messages:", e);
    return false;
  }
}

export async function handleLoadChannelMessages(
  channelId: string,
  limit: number,
): Promise<void> {
  try {
    const messages = await commands.getChannelMessages(channelId, limit);
    const mapped = transformMessages(messages);
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
    setCommunityState("communities", communityId, "members", members.map(transformMember));
  }).catch((e) => {
    console.error("Failed to load community members:", e);
    addToast("Failed to load members", "error");
  });
  // Notify the server we're online in this community
  handleUpdateCommunityPresence(communityId, "online");
  // Refresh community details to ensure myPseudonymKey, roles, categories, and mekGeneration are current
  commands.getCommunityDetails().then((details) => {
    const detail = details.find((c) => c.id === communityId);
    if (detail) {
      setCommunityState("communities", communityId, "myPseudonymKey", detail.myPseudonymKey ?? null);
      setCommunityState("communities", communityId, "mekGeneration", detail.mekGeneration ?? 0);
      setCommunityState("communities", communityId, "myRoleIds", detail.myRoleIds ?? [0, 1]);
      setCommunityState("communities", communityId, "roles", detail.roles ?? []);
      setCommunityState("communities", communityId, "description", detail.description ?? null);
      setCommunityState("communities", communityId, "categories", detail.categories ?? []);
    }
  }).catch((e) => {
    console.error("Failed to refresh community details:", e);
    addToast("Failed to refresh community", "error");
  });
  // Fetch unread counts for all channels in this community
  handleLoadUnreadCounts(communityId);
}

export function handleSelectChannel(channelId: string): void {
  setCommunityState("activeChannel", channelId);

  // Auto-mark-read: find the last message and send mark-read to the server
  const communityId = communityState.activeCommunity;
  if (communityId) {
    const community = communityState.communities[communityId];
    if (community) {
      // Zero unread count locally immediately for responsiveness
      const chIdx = community.channels.findIndex((ch) => ch.id === channelId);
      if (chIdx >= 0 && community.channels[chIdx].unreadCount > 0) {
        setCommunityState("communities", communityId, "channels", chIdx, "unreadCount", 0);
      }

      // Find the last message in the channel to send as read position
      const msgs = communityState.channelMessages[channelId];
      if (msgs && msgs.length > 0) {
        const lastMsg = msgs[msgs.length - 1];
        const lastMessageId = lastMsg.serverMessageId ?? String(lastMsg.id);
        commands.markChannelRead(communityId, channelId, lastMessageId).catch((e) => {
          console.warn("Failed to mark channel read:", e);
        });
      }
    }
  }
}

/// Mark a specific channel as read (explicit call, e.g. from UI button).
export async function handleMarkChannelRead(
  communityId: string,
  channelId: string,
  lastMessageId: string,
): Promise<void> {
  try {
    await commands.markChannelRead(communityId, channelId, lastMessageId);
    const community = communityState.communities[communityId];
    if (community) {
      const chIdx = community.channels.findIndex((ch) => ch.id === channelId);
      if (chIdx >= 0) {
        setCommunityState("communities", communityId, "channels", chIdx, "unreadCount", 0);
      }
    }
  } catch (e) {
    console.error("Failed to mark channel read:", e);
  }
}

/// Fetch unread counts from the backend and update the store.
export async function handleLoadUnreadCounts(communityId: string): Promise<void> {
  try {
    const counts = await commands.getUnreadCounts(communityId);
    const community = communityState.communities[communityId];
    if (community) {
      for (const { channelId, unreadCount } of counts) {
        const chIdx = community.channels.findIndex((ch) => ch.id === channelId);
        if (chIdx >= 0) {
          setCommunityState("communities", communityId, "channels", chIdx, "unreadCount", unreadCount);
        }
      }
    }
  } catch (e) {
    console.error("Failed to load unread counts:", e);
  }
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

// --- Audit log handler ---

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

// --- Community invite handlers ---

export async function handleCreateCommunityInvite(
  communityId: string,
  maxUses?: number,
  expiresInSeconds?: number,
): Promise<{ code: string; manifestKey: string } | null> {
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

// --- Category management handlers ---

export async function handleCreateCategory(
  communityId: string,
  name: string,
): Promise<void> {
  try {
    const { categoryId } = await commands.createCategory(communityId, name);
    const community = communityState.communities[communityId];
    if (community) {
      const maxSortOrder = community.categories.reduce((max, cat) => Math.max(max, cat.sortOrder), -1);
      setCommunityState("communities", communityId, "categories", (cats) => [
        ...cats,
        { id: categoryId, name, sortOrder: maxSortOrder + 1 },
      ]);
    }
  } catch (e) {
    console.error("Failed to create category:", e);
    addToast("Failed to create category", "error");
  }
}

export async function handleDeleteCategory(
  communityId: string,
  categoryId: string,
): Promise<void> {
  try {
    await commands.deleteCategory(communityId, categoryId);
    setCommunityState("communities", communityId, "categories", (cats) =>
      cats.filter((cat) => cat.id !== categoryId),
    );
    // Unset categoryId on channels that belonged to this category
    setCommunityState("communities", communityId, "channels", (chs) =>
      chs.map((ch) => ch.categoryId === categoryId ? { ...ch, categoryId: undefined } : ch),
    );
  } catch (e) {
    console.error("Failed to delete category:", e);
    addToast("Failed to delete category", "error");
  }
}

export async function handleRenameCategory(
  communityId: string,
  categoryId: string,
  newName: string,
): Promise<void> {
  try {
    await commands.renameCategory(communityId, categoryId, newName);
    setCommunityState("communities", communityId, "categories",
      (cat) => cat.id === categoryId,
      "name",
      newName,
    );
  } catch (e) {
    console.error("Failed to rename category:", e);
    addToast("Failed to rename category", "error");
  }
}

export async function handleMoveChannel(
  communityId: string,
  channelId: string,
  categoryId: string | null,
): Promise<void> {
  try {
    await commands.moveChannel(communityId, channelId, categoryId);
    setCommunityState("communities", communityId, "channels",
      (ch) => ch.id === channelId,
      "categoryId",
      categoryId ?? undefined,
    );
  } catch (e) {
    console.error("Failed to move channel:", e);
    addToast("Failed to move channel", "error");
  }
}

export async function handleSetChannelTopic(
  communityId: string,
  channelId: string,
  topic: string,
): Promise<void> {
  try {
    await commands.setChannelTopic(communityId, channelId, topic);
    setCommunityState("communities", communityId, "channels",
      (ch) => ch.id === channelId,
      "topic",
      topic,
    );
  } catch (e) {
    console.error("Failed to set channel topic:", e);
    addToast("Failed to set channel topic", "error");
  }
}

export async function handleReorderChannels(
  communityId: string,
  channelIds: string[],
): Promise<void> {
  try {
    await commands.reorderChannels(communityId, channelIds);
    // Optimistic update — reorder channels to match the specified order
    setCommunityState("communities", communityId, "channels", (chs) => {
      const ordered: typeof chs = [];
      for (const id of channelIds) {
        const ch = chs.find((c) => c.id === id);
        if (ch) ordered.push(ch);
      }
      // Append any channels not in the reorder list (shouldn't happen, but safe)
      for (const ch of chs) {
        if (!channelIds.includes(ch.id)) ordered.push(ch);
      }
      return ordered;
    });
  } catch (e) {
    console.error("Failed to reorder channels:", e);
    addToast("Failed to reorder channels", "error");
  }
}

export async function handleReorderCategories(
  communityId: string,
  categoryIds: string[],
): Promise<void> {
  try {
    await commands.reorderCategories(communityId, categoryIds);
    // Optimistic update — reassign sortOrder based on new ordering
    setCommunityState("communities", communityId, "categories", (cats) =>
      cats.map((cat) => {
        const newOrder = categoryIds.indexOf(cat.id);
        return newOrder >= 0 ? { ...cat, sortOrder: newOrder } : cat;
      }).sort((a, b) => a.sortOrder - b.sortOrder),
    );
  } catch (e) {
    console.error("Failed to reorder categories:", e);
    addToast("Failed to reorder categories", "error");
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

export async function handleSetSlowmode(
  communityId: string,
  channelId: string,
  seconds: number,
): Promise<void> {
  try {
    await commands.setSlowmode(communityId, channelId, seconds);
    // Optimistic update
    const community = communityState.communities[communityId];
    if (community) {
      const idx = community.channels.findIndex((ch) => ch.id === channelId);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "channels", idx, "slowmodeSeconds", seconds || undefined);
      }
    }
  } catch (e) {
    console.error("Failed to set slowmode:", e);
    addToast("Failed to set slowmode", "error");
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
  permissions: string,
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
  permissions: string | null,
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

// --- Reaction handlers ---

export async function handleAddReaction(
  communityId: string,
  channelId: string,
  messageId: string,
  emoji: string,
): Promise<void> {
  try {
    await commands.addReaction(communityId, channelId, messageId, emoji);
  } catch (e) {
    console.error("Failed to add reaction:", e);
    addToast("Failed to add reaction", "error");
  }
}

export async function handleRemoveReaction(
  communityId: string,
  channelId: string,
  messageId: string,
  emoji: string,
): Promise<void> {
  try {
    await commands.removeReaction(communityId, channelId, messageId, emoji);
  } catch (e) {
    console.error("Failed to remove reaction:", e);
    addToast("Failed to remove reaction", "error");
  }
}

// --- Pin handlers ---

export async function handlePinMessage(
  communityId: string,
  channelId: string,
  messageId: string,
): Promise<void> {
  try {
    await commands.pinMessage(communityId, channelId, messageId);
  } catch (e) {
    console.error("Failed to pin message:", e);
    addToast("Failed to pin message", "error");
  }
}

export async function handleUnpinMessage(
  communityId: string,
  channelId: string,
  messageId: string,
): Promise<void> {
  try {
    await commands.unpinMessage(communityId, channelId, messageId);
  } catch (e) {
    console.error("Failed to unpin message:", e);
    addToast("Failed to unpin message", "error");
  }
}

export async function handleGetChannelPins(
  communityId: string,
  channelId: string,
): Promise<{ messageId: string; channelId: string; pinnedBy: string; pinnedAt: number }[]> {
  try {
    return await commands.getChannelPins(communityId, channelId);
  } catch (e) {
    console.error("Failed to get pins:", e);
    return [];
  }
}

export async function handleSendChannelTyping(
  communityId: string,
  channelId: string,
): Promise<void> {
  try {
    await commands.sendChannelTyping(communityId, channelId);
  } catch {
    // Typing indicators are ephemeral — silently ignore failures
  }
}

export async function handleUpdateCommunityPresence(
  communityId: string,
  status: string,
): Promise<void> {
  try {
    await commands.updateCommunityPresence(communityId, status);
  } catch (e) {
    console.error("Failed to update community presence:", e);
  }
}

// --- Community Event CRUD handlers ---

function transformEvent(e: {
  id: string; title: string; description: string; creatorPseudonym: string;
  startTime: number; endTime: number | null; channelId: string | null;
  maxAttendees: number | null; createdAt: number; status: string;
  rsvps: { pseudonymKey: string; status: string }[];
}): CommunityEventType {
  return {
    id: e.id,
    title: e.title,
    description: e.description,
    creatorPseudonym: e.creatorPseudonym,
    startTime: e.startTime,
    endTime: e.endTime,
    channelId: e.channelId,
    maxAttendees: e.maxAttendees,
    createdAt: e.createdAt,
    status: e.status as CommunityEventType["status"],
    rsvps: e.rsvps.map((r) => ({
      pseudonymKey: r.pseudonymKey,
      status: r.status as "going" | "maybe" | "declined",
    })),
  };
}

export async function handleLoadEvents(communityId: string): Promise<void> {
  try {
    const events = await commands.getEvents(communityId);
    setCommunityState("communities", communityId, "events", events.map(transformEvent));
  } catch (e) {
    console.error("Failed to load events:", e);
  }
}

export async function handleCreateEvent(
  communityId: string,
  title: string,
  description: string,
  startTime: number,
  endTime?: number,
  channelId?: string,
  maxAttendees?: number,
): Promise<string | null> {
  try {
    const eventId = await commands.createEvent(communityId, title, description, startTime, endTime, channelId, maxAttendees);
    // Event will arrive via broadcast — but optimistically reload
    await handleLoadEvents(communityId);
    return eventId;
  } catch (e) {
    console.error("Failed to create event:", e);
    addToast("Failed to create event", "error");
    return null;
  }
}

export async function handleEditEvent(
  communityId: string,
  eventId: string,
  title?: string,
  description?: string,
  startTime?: number,
  endTime?: number,
  channelId?: string,
  maxAttendees?: number,
): Promise<void> {
  try {
    await commands.editEvent(communityId, eventId, title, description, startTime, endTime, channelId, maxAttendees);
  } catch (e) {
    console.error("Failed to edit event:", e);
    addToast("Failed to edit event", "error");
  }
}

export async function handleDeleteEvent(
  communityId: string,
  eventId: string,
): Promise<void> {
  try {
    await commands.deleteEvent(communityId, eventId);
  } catch (e) {
    console.error("Failed to delete event:", e);
    addToast("Failed to delete event", "error");
  }
}

export async function handleCancelEvent(
  communityId: string,
  eventId: string,
): Promise<void> {
  try {
    await commands.cancelEvent(communityId, eventId);
  } catch (e) {
    console.error("Failed to cancel event:", e);
    addToast("Failed to cancel event", "error");
  }
}

export async function handleRsvpEvent(
  communityId: string,
  eventId: string,
  status: string,
): Promise<void> {
  try {
    await commands.rsvpEvent(communityId, eventId, status);
  } catch (e) {
    console.error("Failed to RSVP:", e);
    addToast("Failed to update RSVP", "error");
  }
}

// --- Thread handlers ---

export async function handleCreateThread(
  communityId: string,
  channelId: string,
  name: string,
  starterMessageId: string,
): Promise<string | null> {
  try {
    const threadId = await commands.createThread(communityId, channelId, name, starterMessageId);
    // Thread will arrive via broadcast — but optimistically reload
    await handleLoadChannelThreads(communityId, channelId);
    return threadId;
  } catch (e) {
    console.error("Failed to create thread:", e);
    addToast("Failed to create thread", "error");
    return null;
  }
}

export async function handleLoadChannelThreads(
  communityId: string,
  channelId: string,
): Promise<void> {
  try {
    const threads = await commands.getChannelThreads(communityId, channelId);
    setCommunityState("channelThreads", channelId, threads);
  } catch (e) {
    console.error("Failed to load channel threads:", e);
    addToast("Failed to load threads", "error");
  }
}

export async function handleSendThreadMessage(
  communityId: string,
  threadId: string,
  body: string,
): Promise<void> {
  if (!body.trim()) return;
  const trimmed = body.trim();
  const tempId = Date.now();
  const community = communityState.communities[communityId];

  // Optimistic insert with "sending" status
  const message: Message = {
    id: tempId,
    senderId: community?.myPseudonymKey ?? "",
    body: trimmed,
    timestamp: Date.now(),
    isOwn: true,
    status: "sending",
  };
  setCommunityState("threadMessages", threadId, (prev) => [...(prev ?? []), message]);

  try {
    await commands.sendThreadMessage(communityId, threadId, trimmed);
    setCommunityState("threadMessages", threadId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "sent" as const } : m)),
    );
  } catch (e) {
    console.error("Failed to send thread message:", e);
    addToast("Failed to send thread message", "error");
    setCommunityState("threadMessages", threadId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status: "failed" as const } : m)),
    );
  }
}

export async function handleLoadThreadMessages(
  communityId: string,
  threadId: string,
  limit: number,
): Promise<void> {
  try {
    const messages = await commands.getThreadMessages(communityId, threadId, limit);
    const mapped = transformMessages(messages);
    setCommunityState("threadMessages", threadId, mapped);
  } catch (e) {
    console.error("Failed to load thread messages:", e);
    addToast("Failed to load thread messages", "error");
  }
}

export async function handleArchiveThread(
  communityId: string,
  threadId: string,
): Promise<void> {
  try {
    await commands.archiveThread(communityId, threadId);
  } catch (e) {
    console.error("Failed to archive thread:", e);
    addToast("Failed to archive thread", "error");
  }
}

export async function handleUnarchiveThread(
  communityId: string,
  threadId: string,
): Promise<void> {
  try {
    await commands.unarchiveThread(communityId, threadId);
  } catch (e) {
    console.error("Failed to unarchive thread:", e);
    addToast("Failed to unarchive thread", "error");
  }
}

// --- Game server handlers ---

export async function handleAddGameServer(
  communityId: string,
  gameId: string,
  label: string,
  address: string,
): Promise<string | null> {
  try {
    const serverId = await commands.addGameServer(communityId, gameId, label, address);
    // Optimistically add to store
    setCommunityState("gameServers", communityId, (prev) => [
      ...(prev ?? []),
      { id: serverId, gameId, label, address, addedBy: "", createdAt: Date.now() },
    ]);
    return serverId;
  } catch (e) {
    console.error("Failed to add game server:", e);
    addToast("Failed to add game server", "error");
    return null;
  }
}

export async function handleRemoveGameServer(
  communityId: string,
  serverId: string,
): Promise<void> {
  try {
    await commands.removeGameServer(communityId, serverId);
    setCommunityState("gameServers", communityId, (prev) =>
      (prev ?? []).filter((s) => s.id !== serverId),
    );
  } catch (e) {
    console.error("Failed to remove game server:", e);
    addToast("Failed to remove game server", "error");
  }
}

export async function handleLoadGameServers(communityId: string): Promise<void> {
  try {
    const servers = await commands.getGameServers(communityId);
    setCommunityState("gameServers", communityId, servers);
  } catch (e) {
    console.error("Failed to load game servers:", e);
  }
}

export function handleSetCommunityNotifications(
  communityId: string,
  level: "all" | "mentions" | "none",
): void {
  setCommunityState("notificationOverrides", communityId, level);
}

export function getCommunityNotificationLevel(
  communityId: string,
): "all" | "mentions" | "none" {
  return communityState.notificationOverrides[communityId] ?? "all";
}

export async function handleSetNotificationOverride(
  communityId: string, channelId: string, level: "all" | "mentions" | "none"
): Promise<void> {
  const { load } = await import("@tauri-apps/plugin-store");
  setCommunityState("notificationOverrides", `${communityId}:${channelId}`, level);
  const store = await load("notification-settings.json");
  await store.set(`${communityId}:${channelId}`, level);
  await store.save();
}

export async function loadNotificationOverrides(): Promise<void> {
  try {
    const { load } = await import("@tauri-apps/plugin-store");
    const store = await load("notification-settings.json");
    const entries = await store.entries<"all" | "mentions" | "none">();
    for (const [key, value] of entries) {
      setCommunityState("notificationOverrides", key, value);
    }
  } catch {
    // Store may not exist on first run
  }
}

export function subscribeCommunityEventDispatcher(): Promise<UnlistenFn> {
  return subscribeCommunityEvents((event) => {
    if (event.type === "memberJoined") {
      const { communityId, pseudonymKey, displayName, roleIds } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const exists = community.members.some((m) => m.pseudonymKey === pseudonymKey);
        if (!exists) {
          setCommunityState("communities", communityId, "members", (prev) => [
            ...prev,
            transformMember({ pseudonymKey, displayName, roleIds, displayRole: "", status: "online", timeoutUntil: null }),
          ]);
        }
      }
    } else if (event.type === "memberRemoved") {
      const { communityId, pseudonymKey } = event.data;
      setCommunityState("communities", communityId, "members", (prev) =>
        prev.filter((m) => m.pseudonymKey !== pseudonymKey),
      );
    } else if (event.type === "rolesChanged") {
      const { communityId, roles } = event.data;
      if (communityState.communities[communityId]) {
        setCommunityState("communities", communityId, "roles", roles);
      }
    } else if (event.type === "memberRolesChanged") {
      const { communityId, pseudonymKey, roleIds: newRoleIds } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
        if (idx >= 0) {
          setCommunityState("communities", communityId, "members", idx, "roleIds", newRoleIds);
        }
        if (pseudonymKey === community.myPseudonymKey) {
          setCommunityState("communities", communityId, "myRoleIds", newRoleIds);
        }
      }
    } else if (event.type === "memberTimedOut") {
      const { communityId, pseudonymKey, timeoutUntil } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
        if (idx >= 0) {
          setCommunityState("communities", communityId, "members", idx, "timeoutUntil", timeoutUntil);
        }
      }
    } else if (event.type === "channelOverwriteChanged") {
      const { communityId } = event.data;
      if (communityState.communities[communityId]) {
        commands.getCommunityDetails().then((details) => {
          const detail = details.find((d: { id: string }) => d.id === communityId);
          if (detail) {
            setCommunityState("communities", communityId, "roles", detail.roles);
          }
        }).catch(() => {});
      }
    } else if (event.type === "joinAccepted") {
      // Admin peer accepted our join — refresh members and community details
      const { communityId } = event.data;
      commands.getCommunityMembers(communityId).then((members) => {
        setCommunityState("communities", communityId, "members", members.map(transformMember));
      }).catch((e) => {
        console.error("Failed to load members after JoinAccepted:", e);
      });
      commands.getCommunityDetails().then((details) => {
        const detail = details.find((c: { id: string }) => c.id === communityId);
        if (detail) {
          setCommunityState("communities", communityId, "myPseudonymKey", detail.myPseudonymKey ?? null);
          setCommunityState("communities", communityId, "mekGeneration", detail.mekGeneration ?? 0);
          setCommunityState("communities", communityId, "myRoleIds", detail.myRoleIds ?? [0, 1]);
          setCommunityState("communities", communityId, "roles", detail.roles ?? []);
        }
      }).catch(() => {});
      // Now that an admin peer has confirmed our membership, announce our presence.
      // This must happen AFTER JoinAccepted (not in handleJoinCommunity) because
      // admin peers process joins asynchronously — sending presence before
      // the join completes causes "unknown member" rejections.
      handleUpdateCommunityPresence(communityId, "online");
    } else if (event.type === "mekRotated") {
      const { communityId, newGeneration } = event.data;
      if (communityState.communities[communityId]) {
        setCommunityState("communities", communityId, "mekGeneration", newGeneration);
      }
    } else if (event.type === "kicked") {
      const { communityId } = event.data;
      setCommunityState("communities", communityId, undefined!);
      if (communityState.activeCommunity === communityId) {
        setCommunityState("activeCommunity", null);
        setCommunityState("activeChannel", null);
      }
    } else if (event.type === "messageEdited") {
      const { channelId, messageId, newBody, editedAt } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
        if (idx >= 0) {
          setCommunityState("channelMessages", channelId, idx, "body", newBody);
          setCommunityState("channelMessages", channelId, idx, "editedAt", editedAt);
        }
      }
    } else if (event.type === "messageDeleted") {
      const { channelId, messageId } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        setCommunityState("channelMessages", channelId, (prev) =>
          prev.filter((m) => m.serverMessageId !== messageId),
        );
      }
    } else if (event.type === "reactionAdded") {
      const { channelId, messageId, emoji, reactorPseudonym } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
        if (idx >= 0) {
          const msg = msgs[idx];
          const reactions = msg.reactions ?? [];
          const existingIdx = reactions.findIndex((r) => r.emoji === emoji);
          if (existingIdx >= 0) {
            // Add reactor to existing group
            const existing = reactions[existingIdx];
            if (!existing.reactors.includes(reactorPseudonym)) {
              setCommunityState("channelMessages", channelId, idx, "reactions", existingIdx, {
                count: existing.count + 1,
                reactors: [...existing.reactors, reactorPseudonym],
              });
            }
          } else {
            // New reaction group
            setCommunityState("channelMessages", channelId, idx, "reactions", [
              ...reactions,
              { emoji, count: 1, reactors: [reactorPseudonym] },
            ]);
          }
        }
      }
    } else if (event.type === "reactionRemoved") {
      const { channelId, messageId, emoji, reactorPseudonym } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
        if (idx >= 0) {
          const msg = msgs[idx];
          const reactions = msg.reactions ?? [];
          const existingIdx = reactions.findIndex((r) => r.emoji === emoji);
          if (existingIdx >= 0) {
            const existing = reactions[existingIdx];
            const newReactors = existing.reactors.filter((r) => r !== reactorPseudonym);
            if (newReactors.length === 0) {
              // Remove entire reaction group
              setCommunityState("channelMessages", channelId, idx, "reactions",
                reactions.filter((_, i) => i !== existingIdx),
              );
            } else {
              setCommunityState("channelMessages", channelId, idx, "reactions", existingIdx, {
                count: newReactors.length,
                reactors: newReactors,
              });
            }
          }
        }
      }
    } else if (event.type === "messagePinned") {
      // Pin events are informational — UI can show a toast or update pin state
      const { channelId, messageId } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
        if (idx >= 0) {
          setCommunityState("channelMessages", channelId, idx, "pinned", true);
        }
      }
    } else if (event.type === "messageUnpinned") {
      const { channelId, messageId } = event.data;
      const msgs = communityState.channelMessages[channelId];
      if (msgs) {
        const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
        if (idx >= 0) {
          setCommunityState("channelMessages", channelId, idx, "pinned", false);
        }
      }
    } else if (event.type === "channelTyping") {
      const { communityId, channelId, pseudonymKey } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        // Find display name for the typing member
        const member = community.members.find((m) => m.pseudonymKey === pseudonymKey);
        const displayName = member?.displayName ?? truncateKey(pseudonymKey);

        // Track typing users per channel with auto-expire
        const key = `${channelId}:${pseudonymKey}`;
        if (!typingTimers[key]) {
          // Add to typing users for this channel
          setTypingUsers(channelId, (prev) => {
            const existing = prev ?? [];
            if (existing.some((t) => t.pseudonymKey === pseudonymKey)) return existing;
            return [...existing, { pseudonymKey, displayName }];
          });
        } else {
          clearTimeout(typingTimers[key]);
        }
        // Auto-remove after 5 seconds
        typingTimers[key] = window.setTimeout(() => {
          setTypingUsers(channelId, (prev) =>
            (prev ?? []).filter((t) => t.pseudonymKey !== pseudonymKey),
          );
          delete typingTimers[key];
        }, 5000);
      }
    } else if (event.type === "memberPresenceChanged") {
      const { communityId, pseudonymKey, status } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const idx = community.members.findIndex((m) => m.pseudonymKey === pseudonymKey);
        if (idx >= 0) {
          setCommunityState("communities", communityId, "members", idx, "status", status);
          const gameInfo = event.data.gameName
            ? {
                gameName: event.data.gameName,
                gameId: event.data.gameId ?? null,
                startedAt: event.data.elapsedSeconds ?? null,
                serverAddress: event.data.serverAddress ?? null,
              }
            : null;
          setCommunityState("communities", communityId, "members", idx, "gameInfo", gameInfo);
        }
      }
    } else if (event.type === "eventCreated") {
      const { communityId, event: evt } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        setCommunityState("communities", communityId, "events", (prev) => [
          ...(prev ?? []),
          transformEvent(evt),
        ]);
      }
    } else if (event.type === "eventUpdated") {
      const { communityId, event: evt } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const events = community.events ?? [];
        const idx = events.findIndex((e) => e.id === evt.id);
        if (idx >= 0) {
          setCommunityState("communities", communityId, "events", idx, transformEvent(evt));
        } else {
          setCommunityState("communities", communityId, "events", (prev) => [
            ...(prev ?? []),
            transformEvent(evt),
          ]);
        }
      }
    } else if (event.type === "eventDeleted") {
      const { communityId, eventId } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        setCommunityState("communities", communityId, "events", (prev) =>
          (prev ?? []).filter((e) => e.id !== eventId),
        );
      }
    } else if (event.type === "eventRsvpChanged") {
      const { communityId, eventId, pseudonymKey, status } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const events = community.events ?? [];
        const eventIdx = events.findIndex((e) => e.id === eventId);
        if (eventIdx >= 0) {
          const rsvps = events[eventIdx].rsvps;
          const rsvpIdx = rsvps.findIndex((r) => r.pseudonymKey === pseudonymKey);
          if (rsvpIdx >= 0) {
            setCommunityState("communities", communityId, "events", eventIdx, "rsvps", rsvpIdx, "status", status as "going" | "maybe" | "declined");
          } else {
            setCommunityState("communities", communityId, "events", eventIdx, "rsvps", (prev) => [
              ...prev,
              { pseudonymKey, status: status as "going" | "maybe" | "declined" },
            ]);
          }
        }
      }
    } else if (event.type === "threadCreated") {
      const { communityId, thread } = event.data;
      if (communityState.communities[communityId]) {
        const channelId = thread.channelId;
        setCommunityState("channelThreads", channelId, (prev) => [
          ...(prev ?? []),
          thread,
        ]);
      }
    } else if (event.type === "threadMessageReceived") {
      const { communityId, threadId, messageId, senderPseudonym, body, timestamp, replyToId } = event.data;
      const community = communityState.communities[communityId];
      const isOwn = community?.myPseudonymKey === senderPseudonym;

      if (isOwn) {
        // Update optimistic entry instead of duplicating
        setCommunityState("threadMessages", threadId, (prev) => {
          const existing = prev ?? [];
          const optimisticIdx = existing.findIndex((m) => m.status === "sending");
          if (optimisticIdx >= 0) {
            return existing.map((m, i) =>
              i === optimisticIdx
                ? { ...m, serverMessageId: messageId, status: "sent" as const }
                : m,
            );
          }
          return existing;
        });
      } else {
        const newMsg: Message = {
          id: 0,
          senderId: senderPseudonym,
          body,
          timestamp,
          isOwn: false,
          serverMessageId: messageId,
          replyToId: replyToId ?? undefined,
        };
        setCommunityState("threadMessages", threadId, (prev) => [
          ...(prev ?? []),
          newMsg,
        ]);
      }
    } else if (event.type === "threadArchived") {
      const { threadId, archived } = event.data;
      // Update archived flag in all channelThreads entries
      const allChannelIds = Object.keys(communityState.channelThreads);
      for (const channelId of allChannelIds) {
        const threads = communityState.channelThreads[channelId];
        if (threads) {
          const idx = threads.findIndex((t) => t.id === threadId);
          if (idx >= 0) {
            setCommunityState("channelThreads", channelId, idx, "archived", archived);
            break;
          }
        }
      }
    } else if (event.type === "gameServerAdded") {
      const { communityId, server } = event.data;
      setCommunityState("gameServers", communityId, (prev) => [
        ...(prev ?? []),
        server,
      ]);
    } else if (event.type === "gameServerRemoved") {
      const { communityId, serverId } = event.data;
      setCommunityState("gameServers", communityId, (prev) =>
        (prev ?? []).filter((s) => s.id !== serverId),
      );
    } else if (event.type === "eventReminder") {
      const { title, minutesUntilStart } = event.data;
      addToast(`Event "${title}" starts in ${minutesUntilStart} min`, "info");
    } else if (event.type === "channelsUpdated") {
      const { communityId, channels, categories } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        // Preserve unread counts from existing channels
        const unreadMap: Record<string, number> = {};
        for (const ch of community.channels) {
          unreadMap[ch.id] = ch.unreadCount;
        }
        setCommunityState("communities", communityId, "channels",
          channels.map((ch: { id: string; name: string; channelType: string; categoryId?: string; topic?: string; slowmodeSeconds?: number }) => ({
            id: ch.id,
            name: ch.name,
            type: ch.channelType as "text" | "voice" | "announcement",
            unreadCount: unreadMap[ch.id] ?? 0,
            categoryId: ch.categoryId,
            topic: ch.topic,
            slowmodeSeconds: ch.slowmodeSeconds,
          })),
        );
        setCommunityState("communities", communityId, "categories",
          categories.map((cat: { id: string; name: string; sortOrder: number }) => ({
            id: cat.id,
            name: cat.name,
            sortOrder: cat.sortOrder,
          })),
        );
      }
    } else if (event.type === "inviteCreated") {
      const { communityId } = event.data;
      const invite: InviteDto = {
        codeHash: event.data.codeHash,
        createdBy: event.data.createdBy,
        maxUses: event.data.maxUses,
        uses: event.data.uses,
        expiresAt: event.data.expiresAt,
        createdAt: event.data.createdAt,
      };
      // Deduplicate: optimistic insert from handleCreateCommunityInvite may already exist
      setCommunityState("communityInvites", communityId, (prev) => {
        const existing = prev ?? [];
        if (existing.some((inv) => inv.codeHash === invite.codeHash)) return existing;
        // Replace the "pending" optimistic entry if present
        const filtered = existing.filter((inv) => inv.codeHash !== "pending");
        return [invite, ...filtered];
      });
    } else if (event.type === "inviteRevoked") {
      const { communityId, codeHash } = event.data;
      setCommunityState("communityInvites", communityId, (prev) =>
        (prev ?? []).filter((inv) => inv.codeHash !== codeHash),
      );
    } else if (event.type === "inviteUsed") {
      const { communityId, codeHash, newUseCount } = event.data;
      setCommunityState("communityInvites", communityId, (prev) =>
        (prev ?? []).map((inv) =>
          inv.codeHash === codeHash ? { ...inv, uses: newUseCount } : inv,
        ),
      );
    } else if (event.type === "membersRefreshed") {
      const { communityId } = event.data;
      if (communityState.communities[communityId]) {
        commands.getCommunityMembers(communityId).then((members) => {
          setCommunityState("communities", communityId, "members", members.map(transformMember));
        }).catch((e) => {
          console.error(`Failed to refresh members for ${communityId}:`, e);
        });
      }
    } else if (event.type === "memberDiscovered") {
      const { communityId, pseudonymKey, displayName } = event.data;
      const community = communityState.communities[communityId];
      if (community) {
        const exists = community.members.some((m) => m.pseudonymKey === pseudonymKey);
        if (!exists) {
          setCommunityState("communities", communityId, "members", (prev) => [
            ...prev,
            transformMember({ pseudonymKey, displayName, roleIds: [0, 1], displayRole: "", status: "online", timeoutUntil: null }),
          ]);
        }
      }
    } else if (event.type === "voiceJoin") {
      const { channelId, pseudonymKey } = event.data;
      setCommunityState("voiceChannels", channelId, (prev) => {
        const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
        if (state.participants.includes(pseudonymKey)) return state;
        return { ...state, participants: [...state.participants, pseudonymKey] };
      });
    } else if (event.type === "voiceLeave") {
      const { channelId, pseudonymKey } = event.data;
      setCommunityState("voiceChannels", channelId, (prev) => {
        if (!prev) return prev;
        return { ...prev, participants: prev.participants.filter((p) => p !== pseudonymKey) };
      });
    } else if (event.type === "voiceModeSwitch") {
      const { channelId, mode, hostPseudonym } = event.data;
      // Update voice channel state
      setCommunityState("voiceChannels", channelId, (prev) => {
        const state = prev ?? { participants: [], mode: "mesh" as const, hostPseudonym: null };
        return { ...state, mode: mode as "mesh" | "mcu", hostPseudonym };
      });
      // Trigger the Rust set_voice_mode command so our local transport/MCU loop updates
      commands.setVoiceMode(mode, hostPseudonym ?? undefined).catch((e) => {
        console.error("Failed to set voice mode:", e);
      });
    } else if (event.type === "systemMessage") {
      const { communityId, body, timestamp } = event.data;
      const activeChannel = communityState.activeChannel;
      if (activeChannel && communityState.activeCommunity === communityId) {
        const sysMsg: Message = {
          id: Date.now(),
          senderId: "__system__",
          body,
          timestamp,
          isOwn: false,
        };
        setCommunityState("channelMessages", activeChannel, (prev) => [...(prev ?? []), sysMsg]);
      }
    } else if (event.type === "raidAlert") {
      const { communityId, active } = event.data;
      const name = communityState.communities[communityId]?.name ?? communityId;
      if (active) {
        addToast(`Raid alert active in ${name}`, "error");
      } else {
        addToast(`Raid alert cleared in ${name}`, "info");
      }
    } else if (event.type === "channelLockdown") {
      const { communityId, locked } = event.data;
      const name = communityState.communities[communityId]?.name ?? communityId;
      addToast(locked ? `Channels locked in ${name}` : `Channel lockdown lifted in ${name}`, "info");
    } else if (event.type === "onboardingComplete") {
      const { communityId, pseudonymKey, roleIds } = event.data;
      const members = communityState.communities[communityId]?.members ?? [];
      const idx = members.findIndex((m) => m.pseudonymKey === pseudonymKey);
      if (idx >= 0) {
        setCommunityState("communities", communityId, "members", idx, "roleIds", roleIds);
      }
    } else if (event.type === "joinRejected") {
      const { reason } = event.data;
      addToast(`Join rejected: ${reason}`, "error");
    } else if (event.type === "syncComplete") {
      // Sync complete — refresh channel messages from backend
      const { communityId, channelId } = event.data;
      if (communityState.activeCommunity === communityId && communityState.activeChannel === channelId) {
        commands.getChannelMessages(channelId, 100).then((msgs) => {
          setCommunityState("channelMessages", channelId, transformMessages(msgs));
        }).catch((e) => {
          console.error("Failed to refresh messages after sync:", e);
        });
      }
    } else if (event.type === "communityUpdated") {
      const { communityId, name, description } = event.data;
      if (name !== null) {
        setCommunityState("communities", communityId, "name", name);
      }
      if (description !== null) {
        setCommunityState("communities", communityId, "description", description);
      }
    }
  });
}

// ── Onboarding & Welcome Screen ──

export async function handleLoadOnboardingConfig(communityId: string): Promise<void> {
  try {
    const config = await commands.getOnboardingConfig(communityId);
    setCommunityState("communities", communityId, "onboardingConfig", config);
  } catch (e) {
    console.error("Failed to load onboarding config:", e);
  }
}

export async function handleLoadWelcomeScreen(communityId: string): Promise<void> {
  try {
    const screen = await commands.getWelcomeScreen(communityId);
    setCommunityState("communities", communityId, "welcomeScreen", screen);
  } catch (e) {
    console.error("Failed to load welcome screen:", e);
  }
}

export async function handleSubmitOnboarding(
  communityId: string,
  answers: { questionId: string; selectedOptions: string[] }[],
): Promise<boolean> {
  try {
    await commands.submitOnboardingAnswers(communityId, answers);
    setCommunityState("communities", communityId, "onboardingComplete", true);
    return true;
  } catch (e) {
    console.error("Failed to submit onboarding:", e);
    addToast("Failed to complete onboarding", "error");
    return false;
  }
}
