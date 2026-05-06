import { Component, For, Show, createEffect, createMemo, onMount, onCleanup } from "solid-js";
import type { Message } from "../../stores/chat.store";
import {
  bulkSelection,
  clearBulkSelection,
  startBulkSelection,
  toggleBulkSelected,
} from "../../stores/chat.store";
import type { Thread } from "../../stores/community.store";
import MessageBubble from "./MessageBubble";

interface MessageListProps {
  communityId?: string;
  /** Channel id when this list is rendering a community channel — required for bulk-delete UI. */
  channelId?: string;
  messages: Message[];
  ownName: string;
  peerName: string;
  /** Map of pseudonym key → display name for community channels. Overrides peerName when present. */
  memberNames?: Record<string, string>;
  myPseudonymKey?: string | null;
  threads?: Thread[];
  onRetry?: (messageId: number) => void;
  onReply?: (message: Message) => void;
  onReaction?: (messageId: string, emoji: string) => void;
  onRemoveReaction?: (messageId: string, emoji: string) => void;
  onPin?: (messageId: string) => void;
  onCreateThread?: (messageId: string) => void;
  onCreatePoll?: (messageId: string) => void;
  onOpenThread?: (thread: Thread) => void;
  onEdit?: (messageId: string, currentBody: string) => void;
  onDelete?: (messageId: string) => void;
  onVotePoll?: (pollId: string, selectedAnswers: number[]) => void;
  onClosePoll?: (pollId: string) => void;
  onForward?: (messageId: string) => void;
  onLoadOlder?: () => void;
  isLoadingOlder?: boolean;
  /** True when the viewer holds MANAGE_MESSAGES — enables the bulk-delete entry button. */
  canBulkDelete?: boolean;
  /** Invoked with the selected ids when the moderator confirms bulk delete. */
  onBulkDelete?: (messageIds: string[]) => void;
}

const MessageList: Component<MessageListProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let sentinelRef: HTMLDivElement | undefined;
  let observer: IntersectionObserver | undefined;

  const selectionForChannel = createMemo(() => {
    const sel = bulkSelection();
    if (!sel) return null;
    if (props.channelId && sel.channelId === props.channelId) return sel;
    return null;
  });
  const selectionActive = createMemo(() => selectionForChannel() !== null);
  const selectedCount = createMemo(() => selectionForChannel()?.selectedIds.size ?? 0);

  function scrollToBottom(): void {
    requestAnimationFrame(() => {
      if (containerRef) {
        containerRef.scrollTop = containerRef.scrollHeight;
      }
    });
  }

  onMount(() => {
    scrollToBottom();

    // IntersectionObserver for infinite scroll (loads older messages when sentinel is visible)
    if (sentinelRef && containerRef) {
      observer = new IntersectionObserver(
        (entries) => {
          if (entries[0].isIntersecting && props.onLoadOlder && !props.isLoadingOlder) {
            props.onLoadOlder();
          }
        },
        { root: containerRef, threshold: 0.1 },
      );
      observer.observe(sentinelRef);
    }
  });

  onCleanup(() => {
    observer?.disconnect();
  });

  createEffect(() => {
    // Re-scroll when messages change
    props.messages.length;
    scrollToBottom();
  });

  // O(1) lookup map for reply-to resolution instead of O(n) per message
  const messageMap = createMemo(() => {
    const map = new Map<string, Message>();
    for (const m of props.messages) {
      if (m.serverMessageId) map.set(m.serverMessageId, m);
    }
    return map;
  });

  // O(1) lookup: starterMessageId → Thread
  const threadByStarterMessage = createMemo(() => {
    const map = new Map<string, Thread>();
    for (const t of props.threads ?? []) {
      map.set(t.starterMessageId, t);
    }
    return map;
  });

  function handleEnterSelection(): void {
    if (!props.channelId) return;
    startBulkSelection(props.channelId);
  }

  function handleConfirmBulkDelete(): void {
    const sel = selectionForChannel();
    if (!sel) return;
    props.onBulkDelete?.([...sel.selectedIds]);
  }

  return (
    <div
      class="chat-message-area"
      ref={containerRef}
      role="log"
      aria-live="polite"
      aria-relevant="additions"
      aria-atomic="false"
      aria-label={props.channelId ? "Channel messages" : "Direct message conversation"}
    >
      {/* Sentinel for infinite scroll — triggers onLoadOlder when visible */}
      <div ref={sentinelRef} class="messages-scroll-sentinel">
        <Show when={props.isLoadingOlder}>
          <div class="messages-loading-spinner">Loading older messages...</div>
        </Show>
      </div>
      <Show when={selectionActive()}>
        <div class="message-list-selection-toolbar">
          <span class="message-list-selection-count">
            {selectedCount()} selected
          </span>
          <button
            class="message-list-selection-btn message-list-selection-btn-danger"
            disabled={selectedCount() === 0}
            onClick={handleConfirmBulkDelete}
          >
            Delete
          </button>
          <button
            class="message-list-selection-btn"
            onClick={() => clearBulkSelection()}
          >
            Cancel
          </button>
        </div>
      </Show>
      <Show when={!selectionActive() && props.canBulkDelete && props.channelId}>
        <div class="message-list-mod-toolbar">
          <button
            class="message-list-mod-btn"
            onClick={handleEnterSelection}
            title="Enter bulk-delete selection mode"
          >
            Select messages…
          </button>
        </div>
      </Show>
      <For each={props.messages}>
        {(msg) => {
          const thread = () => msg.serverMessageId ? threadByStarterMessage().get(msg.serverMessageId) : undefined;
          const isSelectable = () => selectionActive() && Boolean(msg.serverMessageId);
          const isSelected = () => {
            const sel = selectionForChannel();
            return Boolean(sel && msg.serverMessageId && sel.selectedIds.has(msg.serverMessageId));
          };
          return (
            <MessageBubble
              communityId={props.communityId}
              channelId={props.channelId}
              message={msg}
              senderName={msg.isOwn ? props.ownName : (props.memberNames?.[msg.senderId] ?? props.peerName)}
              myPseudonymKey={props.myPseudonymKey}
              replyToMessage={msg.replyToId ? messageMap().get(msg.replyToId) ?? null : null}
              threadInfo={thread() ? { name: thread()!.name, messageCount: thread()!.messageCount, threadId: thread()!.id } : undefined}
              onRetry={props.onRetry}
              onReply={selectionActive() ? undefined : props.onReply}
              onReaction={!selectionActive() && msg.serverMessageId ? (emoji) => props.onReaction?.(msg.serverMessageId!, emoji) : undefined}
              onRemoveReaction={!selectionActive() && msg.serverMessageId ? (emoji) => props.onRemoveReaction?.(msg.serverMessageId!, emoji) : undefined}
              onPin={selectionActive() ? undefined : props.onPin}
              onCreateThread={!selectionActive() && !thread() ? props.onCreateThread : undefined}
              onCreatePoll={!selectionActive() && !thread() ? props.onCreatePoll : undefined}
              onOpenThread={!selectionActive() && thread() ? () => props.onOpenThread?.(thread()!) : undefined}
              onEdit={selectionActive() ? undefined : props.onEdit}
              onDelete={selectionActive() ? undefined : props.onDelete}
              onVotePoll={selectionActive() ? undefined : props.onVotePoll}
              onClosePoll={selectionActive() ? undefined : props.onClosePoll}
              onForward={selectionActive() ? undefined : props.onForward}
              selectable={isSelectable()}
              selected={isSelected()}
              onToggleSelect={isSelectable() && msg.serverMessageId
                ? () => toggleBulkSelected(msg.serverMessageId!)
                : undefined}
            />
          );
        }}
      </For>
    </div>
  );
};

export default MessageList;
