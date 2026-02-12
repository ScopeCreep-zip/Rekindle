import { commands } from "../ipc/commands";
import { setCommunityState, communityState } from "../stores/community.store";
import { authState } from "../stores/auth.store";
import type { Message } from "../stores/chat.store";

export async function handleCreateCommunity(name: string): Promise<void> {
  try {
    const id = await commands.createCommunity(name);
    // Fetch full community with default channels from backend
    const details = await commands.getCommunityDetails();
    const created = details.find((c) => c.id === id);
    if (created) {
      setCommunityState("communities", id, {
        id: created.id,
        name: created.name,
        channels: created.channels.map((ch) => ({
          id: ch.id,
          name: ch.name,
          type: ch.channelType as "text" | "voice",
          unreadCount: ch.unreadCount,
        })),
        members: [],
        roles: [],
      });
    } else {
      setCommunityState("communities", id, {
        id,
        name,
        channels: [],
        members: [],
        roles: [],
      });
    }
  } catch (e) {
    console.error("Failed to create community:", e);
  }
}

export async function handleJoinCommunity(
  communityId: string,
  name: string,
): Promise<void> {
  try {
    await commands.joinCommunity(communityId);
    setCommunityState("communities", communityId, {
      id: communityId,
      name,
      channels: [],
      members: [],
      roles: [],
    });
  } catch (e) {
    console.error("Failed to join community:", e);
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
  }
}

export function handleSendChannelMessage(
  channelId: string,
  body: string,
): void {
  if (!body.trim()) return;
  const trimmed = body.trim();

  // Optimistic insert
  const message: Message = {
    id: Date.now(),
    senderId: authState.publicKey ?? "",
    body: trimmed,
    timestamp: Date.now(),
    isOwn: true,
  };

  commands.sendChannelMessage(channelId, trimmed).catch((e) => {
    console.error("Failed to send channel message:", e);
  });
  const existing = communityState.channelMessages[channelId];
  if (existing) {
    setCommunityState("channelMessages", channelId, (msgs) => [
      ...msgs,
      message,
    ]);
  } else {
    setCommunityState("channelMessages", channelId, [message]);
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
    setCommunityState("channelMessages", channelId, mapped);
  } catch (e) {
    console.error("Failed to load channel messages:", e);
  }
}

export function handleSelectCommunity(communityId: string): void {
  setCommunityState("activeCommunity", communityId);
  // Fetch members for the selected community
  commands.getCommunityMembers(communityId).then((members) => {
    setCommunityState("communities", communityId, "members", members.map((m) => ({
      publicKey: m.publicKey,
      displayName: m.displayName,
      role: m.role,
      status: m.status,
    })));
  }).catch((e) => {
    console.error("Failed to load community members:", e);
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
  }
}

export async function handleRemoveCommunityMember(
  communityId: string,
  publicKey: string,
): Promise<void> {
  try {
    await commands.removeCommunityMember(communityId, publicKey);
    setCommunityState("communities", communityId, "members", (members) =>
      members.filter((m) => m.publicKey !== publicKey),
    );
  } catch (e) {
    console.error("Failed to remove community member:", e);
  }
}

export async function handleUpdateMemberRole(
  communityId: string,
  publicKey: string,
  role: string,
): Promise<void> {
  try {
    await commands.updateMemberRole(communityId, publicKey, role);
    setCommunityState("communities", communityId, "members",
      (m) => m.publicKey === publicKey,
      "role",
      role,
    );
  } catch (e) {
    console.error("Failed to update member role:", e);
  }
}
