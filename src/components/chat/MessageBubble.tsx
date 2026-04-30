import { Component, Show, createSignal } from "solid-js";
import type { Message } from "../../stores/chat.store";
import ReactionBar from "./ReactionBar";
import EmojiPicker from "./EmojiPicker";
import MessageRichBody from "./MessageRichBody";
import PollCard from "./PollCard";
import ThreadStarter from "./ThreadStarter";
import { formatTimestamp } from "../../utils/formatting";
import { ICON_DOTS, ICON_CHECK, ICON_CLOSE_CIRCLE, ICON_REFRESH, ICON_REPLY, ICON_EMOTICON, ICON_PIN, ICON_THREAD, ICON_PENCIL, ICON_DELETE, ICON_TIMEOUT, ICON_PLUS_BOX } from "../../icons";

interface MessageBubbleProps {
  communityId?: string;
  message: Message;
  senderName: string;
  myPseudonymKey?: string | null;
  replyToMessage?: Message | null;
  threadInfo?: { name: string; messageCount: number; threadId: string };
  onRetry?: (messageId: number) => void;
  onReply?: (message: Message) => void;
  onReaction?: (emoji: string) => void;
  onRemoveReaction?: (emoji: string) => void;
  onPin?: (messageId: string) => void;
  onCreateThread?: (messageId: string) => void;
  onOpenThread?: () => void;
  onEdit?: (messageId: string, currentBody: string) => void;
  onDelete?: (messageId: string) => void;
  onCreatePoll?: (messageId: string) => void;
  onVotePoll?: (pollId: string, selectedAnswers: number[]) => void;
  onClosePoll?: (pollId: string) => void;
}

const MessageBubble: Component<MessageBubbleProps> = (props) => {
  const [showEmojiPicker, setShowEmojiPicker] = createSignal(false);
  const [revealedBlurred, setRevealedBlurred] = createSignal(false);

  const senderClass = () =>
    props.message.isOwn
      ? "chat-message-sender-self"
      : "chat-message-sender-other";

  const statusIcon = () => {
    if (!props.message.isOwn || !props.message.status) return null;
    switch (props.message.status) {
      case "sending": return ICON_DOTS;
      case "sent": return ICON_CHECK;
      case "queued": return ICON_TIMEOUT;
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

  function handleReactionToggle(emoji: string): void {
    const reactions = props.message.reactions ?? [];
    const group = reactions.find((r) => r.emoji === emoji);
    if (group && props.myPseudonymKey && group.reactors.includes(props.myPseudonymKey)) {
      props.onRemoveReaction?.(emoji);
    } else {
      props.onReaction?.(emoji);
    }
  }

  return (
    <div class="chat-message message-enter" data-message-id={props.message.serverMessageId ?? undefined}>
      {/* Reply context */}
      <Show when={props.replyToMessage}>
        {(replyMsg) => (
          <div class="reply-snippet">
            <span class="reply-snippet-sender">{replyMsg().senderId}</span>
            {replyMsg().body.length > 60 ? replyMsg().body.slice(0, 60) + "..." : replyMsg().body}
          </div>
        )}
      </Show>

      <span class={senderClass()}>{props.senderName}</span>
      <span class="chat-message-timestamp">
        {formatTimestamp(props.message.timestamp)}
      </span>
      <Show when={props.message.editedAt}>
        <span class="edit-indicator">(edited)</span>
      </Show>
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

      {/* Action toolbar (appears on hover) */}
      <div class="message-actions">
        <Show when={props.onReply}>
          <button class="message-action-btn" title="Reply" onClick={() => props.onReply?.(props.message)}>
            <span class="nf-icon">{ICON_REPLY}</span>
          </button>
        </Show>
        <Show when={props.onReaction}>
          <button class="message-action-btn" title="React" onClick={() => setShowEmojiPicker(!showEmojiPicker())}>
            <span class="nf-icon">{ICON_EMOTICON}</span>
          </button>
        </Show>
        <Show when={props.onPin && props.message.serverMessageId}>
          <button class="message-action-btn" title={props.message.pinned ? "Unpin" : "Pin"} onClick={() => props.onPin?.(props.message.serverMessageId!)}>
            <span class="nf-icon">{ICON_PIN}</span>
          </button>
        </Show>
        <Show when={props.onCreateThread && props.message.serverMessageId}>
          <button class="message-action-btn" title="Create Thread" onClick={() => props.onCreateThread?.(props.message.serverMessageId!)}>
            <span class="nf-icon">{ICON_THREAD}</span>
          </button>
        </Show>
        <Show when={props.onCreatePoll && props.message.serverMessageId && !props.message.poll}>
          <button class="message-action-btn" title="Create Poll" onClick={() => props.onCreatePoll?.(props.message.serverMessageId!)}>
            <span class="nf-icon">{ICON_PLUS_BOX}</span>
          </button>
        </Show>
        <Show when={props.message.isOwn && props.onEdit && props.message.serverMessageId}>
          <button class="message-action-btn" title="Edit" onClick={() => props.onEdit?.(props.message.serverMessageId!, props.message.body)}>
            <span class="nf-icon">{ICON_PENCIL}</span>
          </button>
        </Show>
        <Show when={props.onDelete && props.message.serverMessageId}>
          <button class="message-action-btn" title="Delete" onClick={() => props.onDelete?.(props.message.serverMessageId!)}>
            <span class="nf-icon">{ICON_DELETE}</span>
          </button>
        </Show>
      </div>

      {/* Emoji picker popup */}
      <Show when={showEmojiPicker()}>
        <EmojiPicker
          communityId={props.communityId}
          onSelect={(emoji) => props.onReaction?.(emoji)}
          mode="reaction"
          onClose={() => setShowEmojiPicker(false)}
        />
      </Show>

      <Show
        when={props.message.decryptionFailed}
        fallback={
          <Show
            when={props.message.automodBlurred && !revealedBlurred()}
            fallback={<MessageRichBody communityId={props.communityId} body={props.message.body} />}
          >
            <div class="message-automod-blur">
              <div class="message-automod-blur-text">Hidden by AutoMod on this client</div>
              <button class="message-automod-reveal-btn" onClick={() => setRevealedBlurred(true)}>
                Reveal
              </button>
            </div>
          </Show>
        }
      >
        <div class="message-decrypt-failed">
          <span class="message-decrypt-failed-icon nf-icon">&#xf023;</span>
          Unable to decrypt — waiting for encryption keys
        </div>
      </Show>

      <Show when={props.message.poll}>
        {(poll) => (
          <PollCard
            poll={poll()}
            onVote={props.onVotePoll ? (selectedAnswers) => props.onVotePoll!(poll().pollId, selectedAnswers) : undefined}
            onClose={props.onClosePoll ? () => props.onClosePoll!(poll().pollId) : undefined}
          />
        )}
      </Show>

      {/* Thread starter badge */}
      <Show when={props.threadInfo}>
        {(info) => (
          <ThreadStarter
            threadName={info().name}
            messageCount={info().messageCount}
            onClick={() => props.onOpenThread?.()}
          />
        )}
      </Show>

      {/* Reactions */}
      <ReactionBar
        communityId={props.communityId}
        reactions={props.message.reactions ?? []}
        myPseudonymKey={props.myPseudonymKey ?? null}
        onToggle={handleReactionToggle}
      />
    </div>
  );
};

export default MessageBubble;
