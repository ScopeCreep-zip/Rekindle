import { Component, For, createEffect, onMount } from "solid-js";
import type { Message } from "../../stores/chat.store";
import MessageBubble from "./MessageBubble";

interface MessageListProps {
  messages: Message[];
  ownName: string;
  peerName: string;
  onRetry?: (messageId: number) => void;
}

const MessageList: Component<MessageListProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;

  function scrollToBottom(): void {
    requestAnimationFrame(() => {
      if (containerRef) {
        containerRef.scrollTop = containerRef.scrollHeight;
      }
    });
  }

  onMount(scrollToBottom);

  createEffect(() => {
    // Re-scroll when messages change
    props.messages.length;
    scrollToBottom();
  });

  return (
    <div class="chat-message-area" ref={containerRef}>
      <For each={props.messages}>
        {(msg) => (
          <MessageBubble
            message={msg}
            senderName={msg.isOwn ? props.ownName : props.peerName}
            onRetry={props.onRetry}
          />
        )}
      </For>
    </div>
  );
};

export default MessageList;
