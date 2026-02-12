import { Component, Show } from "solid-js";
import type { Message } from "../../stores/chat.store";
import { ICON_DOTS, ICON_CHECK, ICON_CLOSE_CIRCLE, ICON_REFRESH } from "../../icons";

interface MessageBubbleProps {
  message: Message;
  senderName: string;
  onRetry?: (messageId: number) => void;
}

function formatTimestamp(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

const MessageBubble: Component<MessageBubbleProps> = (props) => {
  const senderClass = () =>
    props.message.isOwn
      ? "chat-message-sender-self"
      : "chat-message-sender-other";

  const statusIcon = () => {
    if (!props.message.isOwn || !props.message.status) return null;
    switch (props.message.status) {
      case "sending": return ICON_DOTS;
      case "sent": return ICON_CHECK;
      case "failed": return ICON_CLOSE_CIRCLE;
      default: return null;
    }
  };

  const statusClass = () => {
    if (!props.message.status) return "";
    return `message-status message-status-${props.message.status}`;
  };

  function handleRetryClick(): void {
    if (props.onRetry && props.message.status === "failed") {
      props.onRetry(props.message.id);
    }
  }

  return (
    <div class="chat-message message-enter">
      <span class={senderClass()}>{props.senderName}</span>
      <span class="chat-message-timestamp">
        {formatTimestamp(props.message.timestamp)}
      </span>
      <Show when={statusIcon()}>
        <span class={`${statusClass()} nf-icon`}>{statusIcon()}</span>
      </Show>
      <Show when={props.message.status === "failed"}>
        <button
          class="message-retry-btn"
          onClick={handleRetryClick}
          title="Click to retry"
        >
          <span class="nf-icon">{ICON_REFRESH}</span>
        </button>
      </Show>
      <div class="chat-message-body">{props.message.body}</div>
    </div>
  );
};

export default MessageBubble;
