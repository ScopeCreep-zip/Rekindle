import { markChannelReadToLatest } from "../../actions/community/channels";
import { handleLoadThreadMessages } from "../../actions/community/events_threads";
import { handleLoadExpressions } from "../../actions/community/lifecycle";
import { handleLoadChannelMessages } from "../../actions/community/messages";
import { setTypingUsers } from "../../actions/community/shared";
import type { CommunityEvent } from "../../ipc/channels";
import type { UnreadChanged } from "../../ipc/channels/chat_events";
import type {
  ChannelMessageEvent,
  CommunitySubscriptionEvent,
} from "../../ipc/channels/community_subscription_events";
import type { Message } from "../../stores/chat.store";
import { communityState, setCommunityState } from "../../stores/community.store";
import { linkPreviewKey, setLinkPreviews } from "../../stores/link_preview.store";
import { addToast } from "../../stores/toast.store";
import { transformHistoryGap } from "../../utils/transformers";

type NewChannelMessage = Extract<ChannelMessageEvent, { new: unknown }>["new"];

/// Channel-message slice of the community event dispatcher (edits,
/// deletes, reactions, pins, delivery receipts, typing, system msgs,
/// attachment downloads, sync completion). Returns `true` when consumed.
export function reduceMessages(event: CommunityEvent): boolean {
  if (event.type === "linkPreviewReceived") {
    // Architecture §28.8 — sender pre-fetched OpenGraph metadata.
    // Keyed by (channel, message) so MessageBubble renders the card
    // under exactly the message it belongs to.
    const { channelId, messageId, url, title, description, siteName, fetchedAt } = event.data;
    setLinkPreviews(linkPreviewKey(channelId, messageId), {
      url,
      title,
      description,
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
        // The network holds a newer copy of our channel page that lacks this
        // message (plan C7.13); a resend merges it back.
        addToast("Message not delivered. Resend it.", "error");
      }
    }
    return true;
  } else if (event.type === "attachmentDownloaded") {
    applyAttachmentDownloaded(event.data.channelId, event.data.attachmentId);
    return true;
  }
  return false;
}

/// The attachment's row now reads `downloaded`. Attachment ids are
/// unique per channel, so the channel names the message list.
function applyAttachmentDownloaded(channel: string, attachmentId: string): void {
  const messages = communityState.channelMessages[channel];
  if (!messages) return;
  const idx = messages.findIndex((m) => m.attachment?.attachmentId === attachmentId);
  if (idx < 0) return;
  setCommunityState("channelMessages", channel, idx, "attachment", (att) =>
    att ? { ...att, downloaded: true } : att
  );
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

/** Its author's delete takes the message out; a moderator's leaves a
 *  "Removed by" line in its place (plan E3.3 M4). */
function applyDelete(channel: string, messageId: string, removedBy: string | null): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  if (removedBy === null) {
    setCommunityState("channelMessages", channel, (prev) =>
      prev.filter((m) => m.serverMessageId !== messageId)
    );
    return;
  }
  const idx = msgs.findIndex((m) => m.serverMessageId === messageId);
  if (idx < 0) return;
  setCommunityState("channelMessages", channel, idx, (m) => ({
    ...m,
    body: "",
    editedAt: undefined,
    attachment: undefined,
    removedBy,
  }));
}

/// An author's channel outbox gap (`channelHistoryGap`, plan E3.3, D23): the
/// backend stored the marker row; a loaded channel appends it as its history
/// would show it, once per (author, seq). Nothing here infers a gap.
function applyChannelGap(channel: string, sender: string, throughSeq: number): void {
  const msgs = communityState.channelMessages[channel];
  if (!msgs) return;
  if (msgs.some((m) => m.senderId === sender && m.historyGapThrough === throughSeq)) return;
  const marker = transformHistoryGap({ peerKey: sender, throughSeq });
  setCommunityState("channelMessages", channel, (prev) => [...prev, marker]);
}

/// A read reached the author's head (`channelHistoryGapClosed`): the backend
/// removed its markers through `throughSeq` in every channel of the community.
function applyChannelGapClosed(community: string, sender: string, throughSeq: number): void {
  const channels = communityState.communities[community]?.channels ?? [];
  for (const { id } of channels) {
    const msgs = communityState.channelMessages[id];
    if (
      !msgs?.some((m) => m.senderId === sender && (m.historyGapThrough ?? Infinity) <= throughSeq)
    ) {
      continue;
    }
    setCommunityState("channelMessages", id, (prev) =>
      prev.filter(
        (m) => !(m.senderId === sender && (m.historyGapThrough ?? Infinity) <= throughSeq)
      )
    );
  }
}

function applyReactionAdded(
  channel: string,
  messageId: string,
  emoji: string,
  reactor: string
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
  reactor: string
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
      reactions.filter((_, i) => i !== existingIdx)
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

/// A post the Host stored and published (`ChannelMessageEvent::New`, every
/// read path: the outbox read and the slot reads, and our own send) joins
/// the list of a channel this window holds, as the Host gave it, once per
/// message id. Our own posts are shown only from here (plan CP1): the Host
/// publishes our `New` at its local commit, and the send inserts nothing, so
/// a post is one row from the Host's word, its state from `ChannelSendStatus`.
/// A list not loaded yet reads the post with the channel's history (IB2).
function applyNew(post: NewChannelMessage): void {
  const msgs = communityState.channelMessages[post.channel];
  if (!msgs) return;
  if (msgs.some((m) => m.serverMessageId === post.messageId)) return;
  const me = communityState.communities[post.community]?.myPseudonymKey;
  const own = !!me && me.toLowerCase() === post.senderPseudonym.toLowerCase();
  const message: Message = {
    id: Date.now(),
    senderId: post.senderPseudonym,
    body: post.body ?? "",
    decryptionFailed: post.body === null,
    undecryptable: post.undecryptable ?? null,
    automodBlurred: post.automodBlurred,
    timestamp: post.timestamp,
    isOwn: own,
    serverMessageId: post.messageId,
    replyToId: post.replyToMessageId ?? undefined,
    // A live forward draws its label as its history row does (FW1).
    forwardedFromAuthor: post.forwardedFromAuthor ?? null,
    forwardedFromName: post.forwardedFromName,
  };
  setCommunityState("channelMessages", post.channel, (prev) => [...(prev ?? []), message]);
}

/// A channel's unread count is the Host's (`UnreadChanged`), rendered as
/// is. A count in the channel being viewed is read at once.
function applyChannelUnread(u: UnreadChanged): void {
  const ctx = u.context;
  if (typeof ctx !== "object" || !("channel" in ctx)) return;
  const { community: communityId, channel: channelId } = ctx.channel;
  const community = communityState.communities[communityId];
  const chIdx = community?.channels.findIndex((ch) => ch.id === channelId) ?? -1;
  if (chIdx < 0) return;
  const viewing =
    communityState.activeCommunity === communityId && communityState.activeChannel === channelId;
  // A mark names the list's latest stored post; with none loaded the
  // count shows until the history load reads it.
  if (viewing && u.count > 0 && markChannelReadToLatest(communityId, channelId)) {
    return;
  }
  setCommunityState("communities", communityId, "channels", chIdx, "unreadCount", u.count);
}

/// Channel-message and social events on the daemon vocabulary.
/// Our post's delivery state from the Host (`ChannelSendStatus`): `sent` once
/// its slot settled, `failed` when the network superseded our page without it
/// (plan C7.13); a resend merges it back. `awaiting_key` while it waits for
/// the community key, `queued` once released to its write (plan E3.2 S7l);
/// a `failed` one carries the backend's `reason` when its key never came.
function applySendStatus(channel: string, messageId: string, state: string, reason?: string): void {
  // A thread reply's `channel` is its thread (plan FT1): its rows are the thread's.
  const list = communityState.channelMessages[channel]
    ? "channelMessages"
    : communityState.threadMessages[channel]
      ? "threadMessages"
      : null;
  const idx = list
    ? communityState[list][channel].findIndex((m) => m.serverMessageId === messageId)
    : -1;
  if (list === null || idx < 0) {
    // Our own reply whose send answer has not come back yet: its row has no
    // server id to match, so the state waits for that answer (either order).
    earlySendStates.delete(messageId);
    earlySendStates.set(messageId, { state, reason });
    // Only our own posts are announced; one never matched (its window closed)
    // must not be held forever: the oldest goes past a bound.
    if (earlySendStates.size > EARLY_SEND_STATES_MAX) {
      const oldest = earlySendStates.keys().next().value;
      if (oldest !== undefined) earlySendStates.delete(oldest);
    }
    return;
  }
  if (state === "failed") {
    setCommunityState(list, channel, idx, "status", "failed");
    setCommunityState(list, channel, idx, "failureReason", reason);
    addToast(reason ?? "Message not delivered. Resend it.", "error");
  } else if (state === "sent" || state === "queued" || state === "awaiting_key") {
    setCommunityState(list, channel, idx, "status", state);
  }
}

/** A `ChannelSendStatus` that came before its row could be matched, by message id (insertion-ordered). */
const earlySendStates = new Map<string, { state: string; reason?: string }>();

/** How many unmatched send states are held: far above the replies one window has in flight. */
const EARLY_SEND_STATES_MAX = 256;

/**
 * The newest send state the Host announced for `messageId` before its row
 * knew that id (FT1: `ChannelSendStatus` may beat the send's own answer),
 * taken once; undefined when none came first.
 */
export function takeEarlySendState(
  messageId: string
): { state: string; reason?: string } | undefined {
  const early = earlySendStates.get(messageId);
  earlySendStates.delete(messageId);
  return early;
}

export function reduceSubscriptionMessages(event: CommunitySubscriptionEvent): void {
  if ("unreadChanged" in event) {
    applyChannelUnread(event.unreadChanged);
    return;
  }
  if ("channelMessage" in event) {
    const m = event.channelMessage;
    if ("new" in m) {
      applyNew(m.new);
    } else if ("edited" in m) {
      applyEdit(m.edited.channel, m.edited.messageId, m.edited.body, m.edited.editedAt);
    } else if ("deleted" in m) {
      applyDelete(m.deleted.channel, m.deleted.messageId, m.deleted.removedBy);
    } else if ("channelSendStatus" in m) {
      applySendStatus(
        m.channelSendStatus.channel,
        m.channelSendStatus.messageId,
        m.channelSendStatus.state,
        m.channelSendStatus.reason
      );
    } else if ("channelHistoryGap" in m) {
      const g = m.channelHistoryGap;
      applyChannelGap(g.channel, g.senderPseudonym, g.throughSeq);
    } else if ("channelHistoryGapClosed" in m) {
      const g = m.channelHistoryGapClosed;
      applyChannelGapClosed(g.community, g.senderPseudonym, g.throughSeq);
    } else if ("attachmentDownloaded" in m) {
      applyAttachmentDownloaded(
        m.attachmentDownloaded.channel,
        m.attachmentDownloaded.attachmentId
      );
    } else if ("postAdmissionChanged" in m) {
      // CP1: a held post was judged (or a new one held): re-read the
      // channel or thread this window holds; the Host's rows are the truth.
      const a = m.postAdmissionChanged;
      if (a.thread) {
        if (communityState.threadMessages[a.streamId]) {
          void handleLoadThreadMessages(a.community, a.streamId, 100);
        }
      } else if (communityState.channelMessages[a.streamId]) {
        void handleLoadChannelMessages(a.streamId, 100);
      }
    } else if ("expressionAssetReady" in m) {
      // Architecture §18.4 — the eager fetch landed; re-pull the
      // community's expressions so the picker shows the resolved bytes.
      void handleLoadExpressions(m.expressionAssetReady.community);
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
  } else if ("pollChanged" in s) {
    // A poll is the Host's aggregate of its authors' ops (plan E3.3 8c):
    // re-read a channel this window holds to show it.
    if (communityState.channelMessages[s.pollChanged.channel]) {
      void handleLoadChannelMessages(s.pollChanged.channel, 100);
    }
  }
}

/// System-level community signals on the daemon vocabulary.
///
/// Only local observations remain: the gossiped raid alert, lockdown and
/// announcement had no sender and were removed (plan E3.2 S8-E).
export function reduceSubscriptionSystem(event: CommunitySubscriptionEvent): void {
  if (!("system" in event)) return;
  const s = event.system;

  if ("raidDetected" in s) {
    // Architecture §20.6 — this peer's sliding window tripped the
    // policy threshold; surface a moderator banner via the toast
    // layer. The user can then take the spec-listed actions (pause
    // invites, ban floods, raise verification).
    const { joinsInWindow, maxJoinsPerInterval, joinIntervalSeconds } = s.raidDetected;
    addToast(
      `Raid detected: ${joinsInWindow} joins in the last ${joinIntervalSeconds}s ` +
        `(threshold ${maxJoinsPerInterval}). Consider pausing invites.`,
      "error"
    );
    return;
  }

  if ("autoModAlert" in s) {
    addToast(`AutoMod alert: ${s.autoModAlert.ruleName}`, "info");
  }
}

/// Channel typing as the Host states it (plan E3.4 S9): `started` shows a
/// member typing, `stopped` (expiry, their message, or an explicit stop,
/// all the Host's) removes them. The window keeps no timer.
export function reduceSubscriptionTyping(event: CommunitySubscriptionEvent): void {
  if (!("typing" in event)) return;
  const t = event.typing;
  const { context, who } = "started" in t ? t.started : t.stopped;
  if (!("channel" in context)) return;
  const { community, channel } = context.channel;
  const c = communityState.communities[community];
  if (!c) return;

  if ("stopped" in t) {
    setTypingUsers(channel, (prev) => (prev ?? []).filter((x) => x.pseudonymKey !== who));
    return;
  }
  // The typist's name is the Host's (never empty, plan E3.4 S6/S9).
  const displayName = t.started.name;
  setTypingUsers(channel, (prev) => {
    const existing = prev ?? [];
    if (existing.some((x) => x.pseudonymKey === who)) return existing;
    return [...existing, { pseudonymKey: who, displayName }];
  });
}
