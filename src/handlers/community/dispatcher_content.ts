import type { CommunityEvent } from "../../ipc/channels";
import { setCommunityState, communityState } from "../../stores/community.store";
import { commands } from "../../ipc/commands";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import type { InviteDto } from "../../stores/types";
import { transformCommunityDetail } from "../../utils/transformers";
import { showSystemNotification } from "../notification-events.handlers";
import { transformEvent } from "./shared";
import { handleResolveCommunityImageDataUrls } from "./lifecycle";

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
    return true;
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
    return true;
  } else if (event.type === "inviteRevoked") {
    const { communityId, codeHash } = event.data;
    setCommunityState("communityInvites", communityId, (prev) =>
      (prev ?? []).filter((inv) => inv.codeHash !== codeHash),
    );
    return true;
  } else if (event.type === "inviteUsed") {
    const { communityId, codeHash, newUseCount } = event.data;
    setCommunityState("communityInvites", communityId, (prev) =>
      (prev ?? []).map((inv) =>
        inv.codeHash === codeHash ? { ...inv, uses: newUseCount } : inv,
      ),
    );
    return true;
  } else if (event.type === "joinAccepted") {
    // Architecture §7.4 — peer accepted our join request and the
    // MEK has landed in the local cache. Refresh the community
    // detail so the new MEK generation, member registry slot, and
    // governance state propagate into the store, then surface a
    // success toast so the joining user sees the explicit confirmation.
    const { communityId } = event.data;
    addToast("Joined community — encryption keys received", "success");
    void commands.getCommunityDetails().then((details) => {
      const detail = details.find((d) => d.id === communityId);
      if (detail) {
        setCommunityState(
          "communities",
          communityId,
          transformCommunityDetail(detail),
        );
        void handleResolveCommunityImageDataUrls(communityId);
      }
    });
    return true;
  } else if (event.type === "joinRejected") {
    const { reason } = event.data;
    addToast(`Join rejected: ${reason}`, "error");
    return true;
  } else if (event.type === "communityUpdated") {
    const { communityId, name, description, iconHash, bannerHash } = event.data;
    if (name !== null) {
      setCommunityState("communities", communityId, "name", name);
    }
    if (description !== null) {
      setCommunityState("communities", communityId, "description", description);
    }
    // Architecture §32 Phase 5 W15 — when the icon/banner hash
    // changes the cached data URL is stale; clear it and re-resolve
    // through the local cache so the buddy-list icon updates.
    if (iconHash !== null) {
      setCommunityState("communities", communityId, "iconHash", iconHash);
    }
    if (bannerHash !== null) {
      setCommunityState("communities", communityId, "bannerHash", bannerHash);
    }
    if (iconHash !== null || bannerHash !== null) {
      void handleResolveCommunityImageDataUrls(communityId);
    }
    return true;
  }
  return false;
}
