import type { CommunitySubscriptionEvent } from "../../ipc/channels/community_subscription_events";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import { transformEvent } from "../../actions/community/shared";


// ── Daemon vocabulary ──────────────────────────────────────────────
//
// Each body below is the `{ type, data }` case it replaced, moved
// unchanged apart from the destructuring: `communityId` → `community`,
// `pseudonymKey` → `pseudonym`, `status` → `rsvpStatus`, and the
// event/thread/server payloads arriving whole instead of as fragments.

/// Social events on the daemon vocabulary.
export function reduceSubscriptionContent(event: CommunitySubscriptionEvent): void {
  if (!("social" in event)) return;
  const s = event.social;

  if ("eventCreated" in s) {
    const { community, event: evt } = s.eventCreated;
    if (communityState.communities[community]) {
      setCommunityState("communities", community, "events", (prev) => [
        ...(prev ?? []),
        transformEvent(evt),
      ]);
    }
    return;
  }

  if ("eventUpdated" in s) {
    const { community, event: evt } = s.eventUpdated;
    const c = communityState.communities[community];
    if (!c) return;
    const events = c.events ?? [];
    const idx = events.findIndex((e) => e.id === evt.id);
    if (idx >= 0) {
      setCommunityState("communities", community, "events", idx, transformEvent(evt));
    } else {
      setCommunityState("communities", community, "events", (prev) => [
        ...(prev ?? []),
        transformEvent(evt),
      ]);
    }
    return;
  }

  if ("eventDeleted" in s) {
    const { community, eventId } = s.eventDeleted;
    if (communityState.communities[community]) {
      setCommunityState("communities", community, "events", (prev) =>
        (prev ?? []).filter((e) => e.id !== eventId),
      );
    }
    return;
  }

  if ("eventRsvpChanged" in s) {
    const { community, eventId, pseudonym, rsvpStatus } = s.eventRsvpChanged;
    const c = communityState.communities[community];
    if (!c) return;
    const events = c.events ?? [];
    const eventIdx = events.findIndex((e) => e.id === eventId);
    if (eventIdx < 0) return;
    const status = rsvpStatus as "going" | "maybe" | "declined";
    const rsvpIdx = events[eventIdx].rsvps.findIndex((r) => r.pseudonymKey === pseudonym);
    if (rsvpIdx >= 0) {
      setCommunityState(
        "communities", community, "events", eventIdx, "rsvps", rsvpIdx, "status", status,
      );
    } else {
      setCommunityState("communities", community, "events", eventIdx, "rsvps", (prev) => [
        ...prev,
        { pseudonymKey: pseudonym, status },
      ]);
    }
    return;
  }

  if ("threadCreated" in s) {
    const { community, thread } = s.threadCreated;
    if (communityState.communities[community]) {
      setCommunityState("channelThreads", thread.channelId, (prev) => [...(prev ?? []), thread]);
    }
    return;
  }

  if ("threadMessagePosted" in s) {
    const { community, threadId, messageId, senderPseudonym, body, timestamp, replyToId } =
      s.threadMessagePosted;
    const c = communityState.communities[community];
    const existing = communityState.threadMessages[threadId] ?? [];
    // A reply already shown (the event repeats, or a page read held it)
    // neither shows nor counts twice.
    if (existing.some((m) => m.serverMessageId === messageId)) return;
    countThreadReply(threadId, timestamp);

    if (c?.myPseudonymKey === senderPseudonym) {
      // Our own reply, which the Host publishes since the mesh never
      // echoes it (plan E7.13 M76): confirm the bubble waiting for it,
      // or show it when none waits (sent from another window, or the
      // bubble was replaced).
      const waiting = existing.findIndex(
        (m) => m.isOwn && m.status === "sending" && (body === null || m.body === body),
      );
      if (waiting >= 0) {
        setCommunityState("threadMessages", threadId, waiting, (m) => ({
          ...m, serverMessageId: messageId, status: "sent" as const,
        }));
        return;
      }
    }

    // `body` is null when the gossip decoder produced the event: it
    // sees ciphertext and a MEK generation, not plaintext. Rendering
    // an empty bubble is what the old `String::new()` did, so this
    // preserves it rather than dropping the message.
    const newMsg: Message = {
      id: 0,
      senderId: senderPseudonym,
      body: body ?? "",
      timestamp,
      isOwn: c?.myPseudonymKey === senderPseudonym,
      serverMessageId: messageId,
      replyToId: replyToId ?? undefined,
    };
    setCommunityState("threadMessages", threadId, (prev) => [...(prev ?? []), newMsg]);
    return;
  }

  if ("threadArchiveChanged" in s) {
    const { threadId, archived } = s.threadArchiveChanged;
    for (const channelId of Object.keys(communityState.channelThreads)) {
      const threads = communityState.channelThreads[channelId];
      if (!threads) continue;
      const idx = threads.findIndex((t) => t.id === threadId);
      if (idx < 0) continue;
      // An archived thread stays listed, as archived (plan E7.13 walk
      // 18 (b)): it can be read and reopened, as Discord's archived
      // threads can.
      setCommunityState("channelThreads", channelId, idx, "archived", archived);
      break;
    }
    return;
  }

  if ("gameServerAdded" in s) {
    const { community, server } = s.gameServerAdded;
    setCommunityState("gameServers", community, (prev) => [...(prev ?? []), server]);
    return;
  }

  if ("gameServerRemoved" in s) {
    const { community, serverId } = s.gameServerRemoved;
    setCommunityState("gameServers", community, (prev) =>
      (prev ?? []).filter((x) => x.id !== serverId),
    );
    return;
  }

  if ("eventReminder" in s) {
    const { title, minutesUntilStart } = s.eventReminder;
    addToast(`Event "${title}" starts in ${minutesUntilStart} min`, "info");
  }
}

/** Count a new reply on its thread, wherever the window lists it, so the
 * open thread's header and the thread list show it (plan E7.13 M76). */
function countThreadReply(threadId: string, timestamp: number): void {
  for (const channelId of Object.keys(communityState.channelThreads)) {
    const idx = (communityState.channelThreads[channelId] ?? []).findIndex((t) => t.id === threadId);
    if (idx < 0) continue;
    setCommunityState("channelThreads", channelId, idx, (t) => ({
      ...t,
      messageCount: t.messageCount + 1,
      lastMessageAt: Math.max(t.lastMessageAt, timestamp),
    }));
  }
}
