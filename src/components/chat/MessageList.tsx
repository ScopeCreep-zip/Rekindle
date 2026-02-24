import { Component, For, Show, createEffect, createMemo, onMount, onCleanup } from "solid-js";
import type { Message } from "../../stores/chat.store";
import type { Thread } from "../../stores/community.store";
import MessageBubble from "./MessageBubble";

interface MessageListProps {
  messages: Message[];
  ownName: string;
  peerName: string;
  myPseudonymKey?: string | null;
  threads?: Thread[];
  onRetry?: (messageId: number) => void;
  onReply?: (message: Message) => void;
  onReaction?: (messageId: string, emoji: string) => void;
  onRemoveReaction?: (messageId: string, emoji: string) => void;
  onPin?: (messageId: string) => void;
  onCreateThread?: (messageId: string) => void;
  onOpenThread?: (thread: Thread) => void;
  onEdit?: (messageId: string, currentBody: string) => void;
  onDelete?: (messageId: string) => void;
  onLoadOlder?: () => void;
  isLoadingOlder?: boolean;
}

const MessageList: Component<MessageListProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let sentinelRef: HTMLDivElement | undefined;
  let observer: IntersectionObserver | undefined;

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

  return (
    <div class="chat-message-area" ref={containerRef}>
      {/* Sentinel for infinite scroll — triggers onLoadOlder when visible */}
      <div ref={sentinelRef} class="messages-scroll-sentinel">
        <Show when={props.isLoadingOlder}>
          <div class="messages-loading-spinner">Loading older messages...</div>
        </Show>
      </div>
      <For each={props.messages}>
        {(msg) => {
          const thread = () => msg.serverMessageId ? threadByStarterMessage().get(msg.serverMessageId) : undefined;
          return (
            <MessageBubble
              message={msg}
              senderName={msg.isOwn ? props.ownName : props.peerName}
              myPseudonymKey={props.myPseudonymKey}
              replyToMessage={msg.replyToId ? messageMap().get(msg.replyToId) ?? null : null}
              threadInfo={thread() ? { name: thread()!.name, messageCount: thread()!.messageCount, threadId: thread()!.id } : undefined}
              onRetry={props.onRetry}
              onReply={props.onReply}
              onReaction={msg.serverMessageId ? (emoji) => props.onReaction?.(msg.serverMessageId!, emoji) : undefined}
              onRemoveReaction={msg.serverMessageId ? (emoji) => props.onRemoveReaction?.(msg.serverMessageId!, emoji) : undefined}
              onPin={props.onPin}
              onCreateThread={thread() ? undefined : props.onCreateThread}
              onOpenThread={thread() ? () => props.onOpenThread?.(thread()!) : undefined}
              onEdit={props.onEdit}
              onDelete={props.onDelete}
            />
          );
        }}
      </For>
    </div>
  );
};

export default MessageList;
