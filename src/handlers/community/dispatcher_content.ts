import type { CommunityEvent } from "../../ipc/channels";
import { setCommunityState, communityState } from "../../stores/community.store";
import { applyJoinProgress, type JoinStageStatus } from "../../stores/join.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import type { InviteDto } from "../../ipc/commands/dto";
import { transformCommunityDetail } from "../../utils/transformers";
import { showSystemNotification } from "../notification-events.handlers";
import { transformEvent } from "../../actions/community/shared";
import { handleResolveCommunityImageDataUrls } from "../../actions/community/lifecycle";

/// Scheduled-event / thread / game-server / invite / community-update
/// slice of the community event dispatcher. Returns `true` when the
/// event was consumed.
export function reduceContent(event: CommunityEvent): boolean {
  if (event.type === "eventCreated") {
    const { communityId, event: evt } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      setCommunityState("communities", communityId, "events", (prev) => [
        ...(prev ?? []),
        transformEvent(evt),
      ]);
    }
    return true;
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
    return true;
  } else if (event.type === "eventDeleted") {
    const { communityId, eventId } = event.data;
    const community = communityState.communities[communityId];
    if (community) {
      setCommunityState("communities", communityId, "events", (prev) =>
        (prev ?? []).filter((e) => e.id !== eventId),
      );
    }
    return true;
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
    return true;
  } else if (event.type === "threadCreated") {
    const { communityId, thread } = event.data;
    if (communityState.communities[communityId]) {
      const channelId = thread.channelId;
      setCommunityState("channelThreads", channelId, (prev) => [
        ...(prev ?? []),
        thread,
      ]);
    }
    return true;
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
    return true;
  } else if (event.type === "threadArchived") {
    const { threadId, archived } = event.data;
    const allChannelIds = Object.keys(communityState.channelThreads);
    for (const channelId of allChannelIds) {
      const threads = communityState.channelThreads[channelId];
      if (threads) {
        const idx = threads.findIndex((t) => t.id === threadId);
        if (idx >= 0) {
          if (archived) {
            setCommunityState("channelThreads", channelId, (prev) =>
              (prev ?? []).filter((thread) => thread.id !== threadId),
            );
          } else {
            setCommunityState("channelThreads", channelId, idx, "archived", false);
          }
          break;
        }
      }
    }
    return true;
  } else if (event.type === "gameServerAdded") {
    const { communityId, server } = event.data;
    setCommunityState("gameServers", communityId, (prev) => [
      ...(prev ?? []),
      server,
    ]);
    return true;
  } else if (event.type === "gameServerRemoved") {
    const { communityId, serverId } = event.data;
    setCommunityState("gameServers", communityId, (prev) =>
      (prev ?? []).filter((s) => s.id !== serverId),
    );
    return true;
  } else if (event.type === "eventReminder") {
    const { title, minutesUntilStart } = event.data;
    void showSystemNotification(
      "Event Reminder",
      `${title} starts in ${minutesUntilStart} min`,
    );
    addToast(`Event "${title}" starts in ${minutesUntilStart} min`, "info");
    return true;
  }
  return false;
}
