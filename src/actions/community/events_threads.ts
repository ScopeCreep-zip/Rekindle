import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import type { CreateEventRequest } from "../../ipc/commands";
import { transformMessages } from "../../utils/transformers";
import { transformEvent } from "./shared";
import { errorMessage } from "../../utils/error";
import { takeEarlySendState } from "../../handlers/community/dispatcher_messages";

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
  request: CreateEventRequest,
): Promise<string | null> {
  try {
    const eventId = await commands.createEvent(communityId, request);
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

export async function handleCreateThread(
  communityId: string,
  channelId: string,
  name: string,
  starterMessageId: string,
  forumTag?: string | null,
  autoArchiveSeconds?: number,
): Promise<string | null> {
  try {
    const threadId = await commands.createThread(
      communityId,
      channelId,
      name,
      starterMessageId,
      forumTag,
      autoArchiveSeconds,
    );
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
    // Every thread, archived ones flagged: the list shows them apart and
    // an archived one can be opened and reopened (walk 18 (b)).
    const threads = await commands.getChannelThreads(communityId, channelId);
    setCommunityState("channelThreads", channelId, threads);
  } catch (e) {
    console.error("Failed to load channel threads:", e);
    addToast("Failed to load threads", "error");
  }
}

export async function handleCreateForumPost(
  communityId: string,
  channelId: string,
  name: string,
  body: string,
  forumTag?: string | null,
): Promise<string | null> {
  // One Host request creates the thread and queues its starter (plan FT1):
  // created, or refused with nothing created and the Host's reason shown.
  try {
    const created = await commands.createForumPost(communityId, channelId, name, body.trim(), forumTag);
    // The starter is ours and the Host named it: shown at once with its send
    // state (queued until the outbox writes it), so an empty first page of
    // the thread does not hide it; the page and our own event merge into it
    // by its id.
    const early = takeEarlySendState(created.starterMessageId);
    const starter: Message = {
      id: Date.now(),
      senderId: communityState.communities[communityId]?.myPseudonymKey ?? "",
      body: body.trim(),
      timestamp: Date.now(),
      isOwn: true,
      serverMessageId: created.starterMessageId,
      status: (early?.state ?? created.starter) as Message["status"],
      failureReason: early?.reason,
    };
    setCommunityState("threadMessages", created.threadId, (prev) =>
      (prev ?? []).some((m) => m.serverMessageId === created.starterMessageId) ? prev : [...(prev ?? []), starter],
    );
    await handleLoadChannelThreads(communityId, channelId);
    return created.threadId;
  } catch (e) {
    addToast(errorMessage(e), "error");
    return null;
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

  const archivedIn = Object.values(communityState.channelThreads)
    .flat()
    .find((t) => t.id === threadId && t.archived)?.channelId;
  try {
    // The Host's send state (plan FT1): queued in the outbox or sent; a
    // refusal is the only error. A later change arrives as ChannelSendStatus
    // by this id, and one that came before this answer wins over it.
    const sent = await commands.sendThreadMessage(communityId, threadId, trimmed);
    const early = takeEarlySendState(sent.messageId);
    if (early?.state === "failed") addToast(early.reason ?? "Message not delivered. Resend it.", "error");
    setCommunityState("threadMessages", threadId, (msgs) =>
      msgs.map((m) =>
        m.id === tempId
          ? { ...m, serverMessageId: sent.messageId, status: (early?.state ?? sent.sendState) as Message["status"], failureReason: early?.reason }
          : m,
      ),
    );
    // A reply after the archive revives the thread (the Host's listing,
    // `is_thread_archived`): re-read it rather than decide it here.
    if (archivedIn) await handleLoadChannelThreads(communityId, archivedIn);
  } catch (e) {
    console.error("Thread message refused:", e);
    addToast(errorMessage(e), "error");
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
    // The page may answer after a reply was sent or received (walk 14,
    // M76: the late page replaced our own reply's bubble): keep what the
    // page does not hold yet, after it.
    setCommunityState("threadMessages", threadId, (prev) => {
      const held = new Set(mapped.map((m) => m.serverMessageId).filter(Boolean));
      const later = (prev ?? []).filter((m) =>
        m.serverMessageId ? !held.has(m.serverMessageId) : m.isOwn,
      );
      return [...mapped, ...later];
    });
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
