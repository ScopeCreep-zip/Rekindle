import type { CommunityEvent } from "../../ipc/channels";
import type { CommunitySubscriptionEvent } from "../../ipc/channels/community_subscription_events";
import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import { setLinkPreviews } from "../../stores/link_preview.store";
import { truncateKey } from "../../utils/formatting";
import { transformMessages } from "../../utils/transformers";
import { setTypingUsers, typingTimers } from "../../actions/community/shared";

/// Channel-message slice of the community event dispatcher (edits,
/// deletes, reactions, pins, delivery receipts, typing, system msgs,
/// attachment downloads, sync completion). Returns `true` when consumed.
export function reduceMessages(event: CommunityEvent): boolean {
  if (event.type === "linkPreviewReceived") {
    // Architecture §28.8 — sender pre-fetched OpenGraph metadata.
    // Persist keyed by messageId so MessageBubble can render the
    // card under the message body.
    const { messageId, url, title, description, imageUrl, siteName, fetchedAt } = event.data;
    setLinkPreviews(messageId, {
      url,
      title,
      description,
      imageUrl,
      siteName,
      fetchedAt,
    });
    return true;
  } else if (event.type === "channelMessageDelivered") {
    const { channelId, messageId } = event.data;
    const msgs = communityState.channelMessages[channelId];
    if (msgs) {
      const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
      if (idx >= 0) {
        setCommunityState("channelMessages", channelId, idx, "status", "sent");
      }
    }
    return true;
  } else if (event.type === "channelMessageDeliveryFailed") {
    const { channelId, messageId } = event.data;
    const msgs = communityState.channelMessages[channelId];
    if (msgs) {
      const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
      if (idx >= 0) {
        setCommunityState("channelMessages", channelId, idx, "status", "failed");
        addToast("Message delivery failed after retries", "error");
      }
    }
    return true;
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
    return true;
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
    return true;
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
    return true;
  } else if (event.type === "attachmentDownloaded") {
    const { communityId, channelId, attachmentId, localPath } = event.data;
    const messages = communityState.channelMessages[channelId];
    if (!messages) return true;
    const idx = messages.findIndex((m) => m.attachment?.attachmentId === attachmentId);
    if (idx < 0) return true;
    const _ = communityId;
    setCommunityState("channelMessages", channelId, idx, "attachment", (att) =>
      att ? { ...att, localPath } : att,
    );
    return true;
  }
  return false;
}

// ── Daemon vocabulary ──────────────────────────────────────────────
//
// The bodies below are the `{ type, data }` cases they replaced, moved
// verbatim: only the destructuring changed (`channelId` → `channel`,
// `newBody` → `body`). Message edits and deletes are
// `ChannelMessageEvent`; reactions and pins are `SocialEvent`.

function applyEdit(channel: string, messageId: string, body: string, editedAt: number): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
  if (idx >= 0) {
    setCommunityState("channelMessages", channel, idx, "body", body);
    setCommunityState("channelMessages", channel, idx, "editedAt", editedAt);
  }
}

function applyDelete(channel: string, messageId: string): void {
  if (!communityState.channelMessages[channel]) return;
  setCommunityState("channelMessages", channel, (prev) =>
    prev.filter((m) => m.serverMessageId !== messageId),
  );
}

function applyReactionAdded(
  channel: string,
  messageId: string,
  emoji: string,
  reactor: string,
): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
  if (idx < 0) return;
  const reactions = msgs[idx].reactions ?? [];
  const existingIdx = reactions.findIndex((r) => r.emoji === emoji);
  if (existingIdx >= 0) {
    // Add reactor to existing group
    const existing = reactions[existingIdx];
    if (!existing.reactors.includes(reactor)) {
      setCommunityState("channelMessages", channel, idx, "reactions", existingIdx, {
        count: existing.count + 1,
        reactors: [...existing.reactors, reactor],
      });
    }
  } else {
    // New reaction group
    setCommunityState("channelMessages", channel, idx, "reactions", [
      ...reactions,
      { emoji, count: 1, reactors: [reactor] },
    ]);
  }
}

function applyReactionRemoved(
  channel: string,
  messageId: string,
  emoji: string,
  reactor: string,
): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
  if (idx < 0) return;
  const reactions = msgs[idx].reactions ?? [];
  const existingIdx = reactions.findIndex((r) => r.emoji === emoji);
  if (existingIdx < 0) return;
  const existing = reactions[existingIdx];
  const newReactors = existing.reactors.filter((r) => r !== reactor);
  if (newReactors.length === 0) {
    // Remove entire reaction group
    setCommunityState(
      "channelMessages",
      channel,
      idx,
      "reactions",
      reactions.filter((_, i) => i !== existingIdx),
    );
  } else {
    setCommunityState("channelMessages", channel, idx, "reactions", existingIdx, {
      count: newReactors.length,
      reactors: newReactors,
    });
  }
}

function applyPinned(channel: string, messageId: string, pinned: boolean): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
  if (idx >= 0) {
    setCommunityState("channelMessages", channel, idx, "pinned", pinned);
  }
}

/// Channel-message and social events on the daemon vocabulary.
export function reduceSubscriptionMessages(event: CommunitySubscriptionEvent): void {
  if ("channelMessage" in event) {
    const m = event.channelMessage;
    if ("edited" in m) {
      applyEdit(m.edited.channel, m.edited.messageId, m.edited.body, m.edited.editedAt);
    } else if ("deleted" in m) {
      applyDelete(m.deleted.channel, m.deleted.messageId);
    }
    return;
  }

  if (!("social" in event)) return;
  const s = event.social;

  if ("reactionAdded" in s) {
    const r = s.reactionAdded;
    applyReactionAdded(r.channel, r.messageId, r.emoji, r.reactorPseudonym);
  } else if ("reactionRemoved" in s) {
    const r = s.reactionRemoved;
    applyReactionRemoved(r.channel, r.messageId, r.emoji, r.reactorPseudonym);
  } else if ("messagePinned" in s) {
    // Pin events are informational — UI can show a toast or update pin state
    applyPinned(s.messagePinned.channel, s.messagePinned.messageId, true);
  } else if ("messageUnpinned" in s) {
    applyPinned(s.messageUnpinned.channel, s.messageUnpinned.messageId, false);
  }
}
