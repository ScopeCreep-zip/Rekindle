import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import type { Channel } from "../../stores/community.store";
import { refreshStageHandRaises } from "./shared";

export async function handleCreateChannel(
  communityId: string,
  name: string,
  channelType: string,
  categoryId?: string,
  parentVoiceChannelId?: string,
): Promise<string> {
  try {
    const channelId = await commands.createChannel(
      communityId,
      name,
      channelType,
      categoryId,
      parentVoiceChannelId,
    );
    setCommunityState("communities", communityId, "channels", (chs) => [
      ...chs,
      {
        id: channelId,
        name,
        type: channelType as Channel["type"],
        unreadCount: 0,
        categoryId,
        parentVoiceChannelId: parentVoiceChannelId ?? null,
      },
    ]);
    return channelId;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Failed to create channel";
    console.error("Failed to create channel:", e);
    addToast(msg, "error");
    throw e;
  }
}

// Debounce active-channel publishing: a user scrubbing through channels
// shouldn't trigger a registry presence write per channel. Only the
// channel they settle on (~400ms) is announced to peers.
let activeChannelTimer: ReturnType<typeof setTimeout> | undefined;

function publishActiveChannel(communityId: string, channelId: string, kind: string): void {
  if (activeChannelTimer) clearTimeout(activeChannelTimer);
  activeChannelTimer = setTimeout(() => {
    commands.setActiveChannel(communityId, channelId, kind).catch((e) => {
      console.warn("Failed to publish active channel:", e);
    });
  }, 400);
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

      // Tell peers which channel we're focused on (debounced). Voice
      // join/leave publishes its own Voice location separately.
      if (chIdx >= 0) {
        const chType = community.channels[chIdx].type;
        const kind = chType === "voice" || chType === "stage" ? "voice" : "text";
        publishActiveChannel(communityId, channelId, kind);
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

  const activeCommunityId = communityState.activeCommunity;
  if (activeCommunityId) {
    void refreshStageHandRaises(activeCommunityId, channelId);
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

export async function handleSetChannelForumTags(
  communityId: string,
  channelId: string,
  forumTags: string[],
): Promise<void> {
  try {
    await commands.setChannelForumTags(communityId, channelId, forumTags);
    setCommunityState("communities", communityId, "channels",
      (ch) => ch.id === channelId,
      "forumTags",
      forumTags.length > 0 ? forumTags : undefined,
    );
  } catch (e) {
    console.error("Failed to set forum tags:", e);
    addToast("Failed to set forum tags", "error");
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
