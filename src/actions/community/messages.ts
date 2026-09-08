import { commands } from "../../ipc/commands";
import { setCommunityState, communityState } from "../../stores/community.store";
import { authState } from "../../stores/auth.store";
import { addToast } from "../../stores/toast.store";
import type { Message } from "../../stores/chat.store";
import { clearBulkSelection } from "../../stores/chat.store";
import { transformMessages } from "../../utils/transformers";

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
    const status = result.status === "queued" ? ("queued" as const) : ("sent" as const);
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === tempId ? { ...m, status, serverMessageId: result.messageId } : m)),
    );
    if (result.status === "queued") {
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
    const result = await commands.sendChannelMessage(channelId, message.body);
    setCommunityState("channelMessages", channelId, (msgs) =>
      msgs.map((m) => (m.id === messageId ? { ...m, status: result.status === "queued" ? "queued" as const : "sent" as const, serverMessageId: result.messageId } : m)),
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

export async function handleSendVoiceMessage(
  communityId: string,
  channelId: string,
  opusBytesB64: string,
  durationMs: number,
  waveformB64: string,
): Promise<boolean> {
  try {
    await commands.sendVoiceMessage(
      communityId,
      channelId,
      opusBytesB64,
      durationMs,
      waveformB64,
    );
    return true;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Voice message failed";
    console.error("Voice message failed:", e);
    addToast(msg, "error");
    return false;
  }
}

export async function handleForwardChannelMessage(
  sourceCommunityId: string,
  sourceChannelId: string,
  sourceMessageId: string,
  destCommunityId: string,
  destChannelId: string,
): Promise<boolean> {
  try {
    await commands.forwardChannelMessage(
      sourceCommunityId,
      sourceChannelId,
      sourceMessageId,
      destCommunityId,
      destChannelId,
    );
    addToast("Message forwarded", "success");
    return true;
  } catch (e) {
    const msg = typeof e === "string" ? e : "Forward failed";
    console.error("Forward failed:", e);
    addToast(msg, "error");
    return false;
  }
}

/**
 * Bulk-delete a set of channel messages as a moderator. The backend writes
 * one `GovernanceEntry::AdminDelete` per id (capped at 100), gossips
 * `MessageDeleted` for each, purges the local SQLite row, and emits a local
 * `MessageDeleted` event so the UI updates immediately. Per
 * `feedback_no_fallback.md` we do NOT optimistically remove from the store —
 * the backend's local emit is the single source of truth.
 */
export async function handleBulkDeleteChannelMessages(
  communityId: string,
  channelId: string,
  messageIds: string[],
  reason?: string,
): Promise<void> {
  if (messageIds.length === 0) {
    clearBulkSelection();
    return;
  }
  try {
    const deleted = await commands.bulkDeleteChannelMessages(
      communityId,
      channelId,
      messageIds,
      reason,
    );
    clearBulkSelection();
    addToast(
      `Deleted ${deleted} message${deleted === 1 ? "" : "s"}`,
      "success",
    );
  } catch (e) {
    const msg = typeof e === "string" ? e : "Bulk delete failed";
    console.error("Bulk delete failed:", e);
    addToast(msg, "error");
  }
}

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

export async function handleVotePoll(
  communityId: string,
  channelId: string,
  pollId: string,
  selectedAnswers: number[],
): Promise<void> {
  try {
    await commands.votePoll(communityId, channelId, pollId, selectedAnswers);
    await handleLoadChannelMessages(channelId, 100);
  } catch (e) {
    console.error("Failed to vote in poll:", e);
    addToast("Failed to vote in poll", "error");
  }
}

export async function handleCreatePoll(
  communityId: string,
  channelId: string,
  messageId: string,
  question: string,
  answers: string[],
  multiSelect: boolean,
  durationSeconds?: number,
): Promise<string | null> {
  try {
    const pollId = await commands.createPoll(
      communityId,
      channelId,
      messageId,
      question,
      answers,
      multiSelect,
      durationSeconds,
    );
    await handleLoadChannelMessages(channelId, 100);
    return pollId;
  } catch (e) {
    console.error("Failed to create poll:", e);
    addToast("Failed to create poll", "error");
    return null;
  }
}

export async function handleClosePoll(
  communityId: string,
  channelId: string,
  pollId: string,
): Promise<void> {
  try {
    await commands.closePoll(communityId, channelId, pollId);
    await handleLoadChannelMessages(channelId, 100);
  } catch (e) {
    console.error("Failed to close poll:", e);
    addToast("Failed to close poll", "error");
  }
}

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
