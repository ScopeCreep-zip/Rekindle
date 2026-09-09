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

/// System-level community signals on the daemon vocabulary.
///
/// Bodies moved from the `{ type, data }` cases they replaced;
/// `SystemMessage` became `Announcement` and `SyncComplete` became
/// `SyncReceived`, which Tier 1 already had.
export function reduceSubscriptionSystem(event: CommunitySubscriptionEvent): void {
  if (!("system" in event)) return;
  const s = event.system;

  if ("announcement" in s) {
    const { community, body, timestamp } = s.announcement;
    const activeChannel = communityState.activeChannel;
    if (activeChannel && community !== null && communityState.activeCommunity === community) {
      const sysMsg: Message = {
        id: Date.now(),
        senderId: "__system__",
        body,
        timestamp,
        isOwn: false,
      };
      setCommunityState("channelMessages", activeChannel, (prev) => [...(prev ?? []), sysMsg]);
    }
    return;
  }

  if ("syncReceived" in s) {
    // Sync complete — refresh channel messages from backend
    const { community, channel } = s.syncReceived;
    if (
      communityState.activeCommunity === community &&
      communityState.activeChannel === channel
    ) {
      commands
        .getChannelMessages(channel, 100)
        .then((msgs) => {
          setCommunityState("channelMessages", channel, transformMessages(msgs));
        })
        .catch((e) => {
          console.error("Failed to refresh messages after sync:", e);
        });
    }
    return;
  }

  if ("raidDetected" in s) {
    // Architecture §20.6 — this peer's sliding window tripped the
    // policy threshold; surface a moderator banner via the toast
    // layer. The user can then take the spec-listed actions (pause
    // invites, ban floods, raise verification).
    const { joinsInWindow, maxJoinsPerInterval, joinIntervalSeconds } = s.raidDetected;
    addToast(
      `Raid detected: ${joinsInWindow} joins in the last ${joinIntervalSeconds}s ` +
        `(threshold ${maxJoinsPerInterval}). Consider pausing invites.`,
      "error",
    );
    return;
  }

  if ("autoModAlert" in s) {
    addToast(`AutoMod alert: ${s.autoModAlert.ruleName}`, "info");
    return;
  }

  if ("raidAlert" in s) {
    // Architecture §17.4 — raid alert lives in store; CommunityWindow
    // renders a banner overlay (`role="alert"`) for higher visibility
    // than a transient toast. The flag persists until the backend
    // emits `active: false` (or the user clears it client-side via
    // `dismissRaidAlertLocal`).
    const { community, active } = s.raidAlert;
    setCommunityState("communities", community, "raidAlertActive", active);
    return;
  }

  if ("channelLockdown" in s) {
    const { community, locked } = s.channelLockdown;
    const name = communityState.communities[community]?.name ?? community;
    addToast(
      locked ? `Channels locked in ${name}` : `Channel lockdown lifted in ${name}`,
      "info",
    );
  }
}
