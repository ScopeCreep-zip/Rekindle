import { Component, Show, createSignal } from "solid-js";
import type { Message } from "../../stores/chat.store";
import { createMemo } from "solid-js";
import { Popover } from "@kobalte/core/popover";
import ReactionBar from "./ReactionBar";
import EmojiPicker from "./EmojiPicker";
import MessageRichBody from "./MessageRichBody";
import { communityState } from "../../stores/community.store";
import PollCard from "./PollCard";
import ThreadStarter from "./ThreadStarter";
import AttachmentDisplay from "./AttachmentDisplay";
import VoiceMessagePlayer from "./VoiceMessagePlayer";
import { FLAG_VOICE_MESSAGE } from "../../stores/chat.store";
import { formatTimestamp } from "../../utils/formatting";
import { messageAnnouncement } from "../../utils/message_announcement";
import { linkPreviewKey, linkPreviews } from "../../stores/link_preview.store";
import { commands } from "../../ipc/commands";
import { ICON_DOTS, ICON_CHECK, ICON_CLOSE_CIRCLE, ICON_REFRESH, ICON_REPLY, ICON_EMOTICON, ICON_PIN, ICON_THREAD, ICON_PENCIL, ICON_DELETE, ICON_TIMEOUT, ICON_PLUS_BOX, ICON_FORWARD, ICON_CHECK_ALL, ICON_EYE} from "../../icons";

interface MessageBubbleProps {
  communityId?: string;
  message: Message;
  senderName: string;
  myPseudonymKey?: string | null;
  replyToMessage?: Message | null;
  threadInfo?: { name: string; messageCount: number; threadId: string; archived: boolean };
  onRetry?: (messageId: number) => void;
  onDiscard?: (messageId: number) => void;
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
  /** Bulk-delete selection mode: render checkbox + suppress action toolbar. */
  selectable?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
  /** Open forward-message dialog for this message. */
  onForward?: (messageId: string) => void;
  /** Channel id this message belongs to (used by attachment download flow). */
  channelId?: string;
}


const MessageBubble: Component<MessageBubbleProps> = (props) => {
  const [showEmojiPicker, setShowEmojiPicker] = createSignal(false);
  const [revealedBlurred, setRevealedBlurred] = createSignal(false);
  // Architecture §28.8 — keyed by (channel, serverMessageId); absent
  // when the sender hadn't fetched a preview (no URL, missing
  // permission, OpenGraph fetch failed, etc.).
  const preview = createMemo(() => {
    const id = props.message.serverMessageId;
    if (!id || !props.channelId) return undefined;
    return linkPreviews[linkPreviewKey(props.channelId, id)];
  });

  const senderClass = () =>
    props.message.isOwn
      ? "chat-message-sender-self"
      : "chat-message-sender-other";

  const statusIcon = () => {
    if (!props.message.isOwn || !props.message.status) return null;
    switch (props.message.status) {
      case "sending": return ICON_DOTS;
      case "sent": return ICON_CHECK;
      case "delivered": return ICON_CHECK_ALL;
      case "read": return ICON_EYE;
      case "awaiting_key":
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

  function handleDiscardClick(): void {
    if (props.onDiscard && props.message.status === "failed") {
      props.onDiscard(props.message.id);
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

  function handleBubbleClick(): void {
    if (props.selectable && props.onToggleSelect) {
      props.onToggleSelect();
    }
  }

  /** A voice message: the Host decodes its duration and waveform from the
   *  audio (plan E5.1), so nothing is read from the message body. */
  const isVoice = createMemo(
    () => props.message.flags != null && (props.message.flags & FLAG_VOICE_MESSAGE) !== 0,
  );

  /** The moderator who removed it, by their name in this community when
   *  we hold it (plan E3.3 M4). */
  const removedByName = (): string => {
    const by = props.message.removedBy ?? "";
    const community = props.communityId ? communityState.communities[props.communityId] : undefined;
    return community?.members?.find((m) => m.pseudonymKey === by)?.displayName ?? `${by.slice(0, 8)}…`;
  };

  // Architecture §32 a11y — the bubble's accessible name: sender, time and
  // body (shared with the chat window's new-message announcer).
  const ariaAnnouncement = (): string =>
    messageAnnouncement(props.message, props.senderName, removedByName);

  return (
    <div
      class={`chat-message message-enter ${props.selectable ? "chat-message-selectable" : ""} ${props.selected ? "chat-message-selected" : ""}`}
      data-message-id={props.message.serverMessageId ?? undefined}
      role="article"
      aria-label={ariaAnnouncement()}
      onClick={handleBubbleClick}
    >
      <Show when={props.selectable}>
        <input
          type="checkbox"
          class="chat-message-select-checkbox"
          checked={props.selected ?? false}
          onChange={() => props.onToggleSelect?.()}
          onClick={(e) => e.stopPropagation()}
        />
      </Show>
      {/* Forwarded indicator */}
      <Show when={props.message.forwardedFromAuthor}>
        {(author) => (
          <div class="message-forwarded-header">
            <span class="nf-icon message-forwarded-icon">{ICON_FORWARD}</span>
            Forwarded from {author().slice(0, 12)}…
          </div>
        )}
      </Show>
      {/* Reply context */}
      <Show when={props.replyToMessage}>
        {(replyMsg) => (
          <div class="reply-snippet">
            <span class="reply-snippet-sender">{replyMsg().senderId}</span>
            {replyMsg().body.length > 60 ? replyMsg().body.slice(0, 60) + "..." : replyMsg().body}
          </div>
        )}
      </Show>

      <div class="chat-message-header">
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
        <Show when={props.message.isOwn && props.message.status === "awaiting_key"}>
          <span class="message-status-note">Waiting for the community key</span>
        </Show>
        <Show when={props.message.isOwn && props.message.status === "failed" && props.message.failureReason}>
          <span class="message-status-note message-status-note-failed">{props.message.failureReason}</span>
        </Show>
        <Show when={props.message.status === "failed"}>
          <button
            class="message-retry-btn"
            onClick={handleRetryClick}
            title="Click to retry"
            aria-label="Retry sending message"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_REFRESH}</span>
          </button>
          <Show when={props.onDiscard}>
            <button
              class="message-discard-btn"
              onClick={handleDiscardClick}
              title="Discard"
              aria-label="Discard failed message"
            >
              <span class="nf-icon" aria-hidden="true">{ICON_DELETE}</span>
            </button>
          </Show>
        </Show>
      </div>

      {/* Action toolbar (appears on hover) */}
      <div class="message-actions">
        <Show when={props.onReply}>
          <button class="message-action-btn" title="Reply" aria-label="Reply" onClick={() => props.onReply?.(props.message)}>
            <span class="nf-icon" aria-hidden="true">{ICON_REPLY}</span>
          </button>
        </Show>
        <Show when={props.onReaction}>
          {/* Plan §Failure 3 — Kobalte Popover with Popover.Portal so the
           * picker escapes the chat-message-area's `overflow-y: auto`
           * clip and the `.chat-message` stacking context. Mirrors the
           * MemberList Popover.Portal pattern. */}
          <Popover open={showEmojiPicker()} onOpenChange={setShowEmojiPicker}>
            <Popover.Trigger
              as="button"
              class="message-action-btn"
              title="React"
              aria-label="Add reaction"
            >
              <span class="nf-icon" aria-hidden="true">{ICON_EMOTICON}</span>
            </Popover.Trigger>
            <Popover.Portal>
              <Popover.Content class="emoji-picker-popover">
                <EmojiPicker
                  communityId={props.communityId}
                  onSelect={(emoji) => props.onReaction?.(emoji)}
                  mode="reaction"
                  onClose={() => setShowEmojiPicker(false)}
                />
              </Popover.Content>
            </Popover.Portal>
          </Popover>
        </Show>
        <Show when={props.onPin && props.message.serverMessageId}>
          <button
            class="message-action-btn"
            title={props.message.pinned ? "Unpin" : "Pin"}
            aria-label={props.message.pinned ? "Unpin message" : "Pin message"}
            onClick={() => props.onPin?.(props.message.serverMessageId!)}
          >
            <span class="nf-icon" aria-hidden="true">{ICON_PIN}</span>
          </button>
        </Show>
        <Show when={props.onCreateThread && props.message.serverMessageId}>
          <button class="message-action-btn" title="Create Thread" aria-label="Create thread" onClick={() => props.onCreateThread?.(props.message.serverMessageId!)}>
            <span class="nf-icon" aria-hidden="true">{ICON_THREAD}</span>
          </button>
        </Show>
        <Show when={props.onCreatePoll && props.message.serverMessageId && !props.message.poll}>
          <button class="message-action-btn" title="Create Poll" aria-label="Create poll" onClick={() => props.onCreatePoll?.(props.message.serverMessageId!)}>
            <span class="nf-icon" aria-hidden="true">{ICON_PLUS_BOX}</span>
          </button>
        </Show>
        <Show when={props.onForward && props.message.serverMessageId}>
          <button class="message-action-btn" title="Forward" aria-label="Forward message" onClick={() => props.onForward?.(props.message.serverMessageId!)}>
            <span class="nf-icon" aria-hidden="true">{ICON_FORWARD}</span>
          </button>
        </Show>
        <Show when={props.message.isOwn && props.onEdit && props.message.serverMessageId}>
          <button class="message-action-btn" title="Edit" aria-label="Edit message" onClick={() => props.onEdit?.(props.message.serverMessageId!, props.message.body)}>
            <span class="nf-icon" aria-hidden="true">{ICON_PENCIL}</span>
          </button>
        </Show>
        <Show when={props.onDelete && props.message.serverMessageId}>
          <button class="message-action-btn" title="Delete" aria-label="Delete message" onClick={() => props.onDelete?.(props.message.serverMessageId!)}>
            <span class="nf-icon" aria-hidden="true">{ICON_DELETE}</span>
          </button>
        </Show>
      </div>

      <Show
        when={!props.message.removedBy}
        fallback={<div class="message-removed">Removed by {removedByName()}</div>}
      >
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
      </Show>

      {/* Architecture §28.8 — sender-fetched OpenGraph card. The
       * sender's IP is exposed to the destination site at fetch time;
       * receivers always reader-validate the EMBED_LINKS permission
       * via `services/community/link_previews.rs::handle_incoming_link_preview`. */}
      <Show when={preview()}>
        {(p) => (
          <button
            type="button"
            class="message-link-preview"
            onClick={() => void commands.openExternalUrl(p().url)}
            aria-label={`Link preview: ${p().title ?? p().url}`}
          >
            <div class="message-link-preview-body">
              <Show when={p().siteName}>
                <div class="message-link-preview-site">{p().siteName}</div>
              </Show>
              <Show when={p().title}>
                <div class="message-link-preview-title">{p().title}</div>
              </Show>
              <Show when={p().description}>
                <div class="message-link-preview-description">{p().description}</div>
              </Show>
            </div>
          </button>
        )}
      </Show>

      <Show when={props.message.attachment && isVoice()}>
        <Show when={props.communityId && props.channelId}>
          <VoiceMessagePlayer
            communityId={props.communityId!}
            channelId={props.channelId!}
            attachment={props.message.attachment!}
          />
        </Show>
      </Show>
      <Show when={props.message.attachment && !isVoice() ? props.message.attachment : null}>
        {(att) => (
          <Show when={props.communityId && props.channelId}>
            <AttachmentDisplay
              communityId={props.communityId!}
              channelId={props.channelId!}
              attachment={att()}
            />
          </Show>
        )}
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
            archived={info().archived}
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
