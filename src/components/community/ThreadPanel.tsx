import { Component, Show, For, createSignal, createEffect } from "solid-js";
import type { Thread } from "../../stores/community.store";
import type { Message } from "../../stores/chat.store";
import MessageBubble from "../chat/MessageBubble";
import { ICON_CLOSE, ICON_THREAD, ICON_ARCHIVE } from "../../icons";

interface ThreadPanelProps {
  thread: Thread | null;
  communityId: string;
  messages: Message[];
  onClose: () => void;
  onSend: (communityId: string, threadId: string, body: string) => void;
  onArchive: (communityId: string, threadId: string) => void;
  onUnarchive: (communityId: string, threadId: string) => void;
  onReply?: (message: Message) => void;
  onReaction?: (messageId: string, emoji: string) => void;
  onRemoveReaction?: (messageId: string, emoji: string) => void;
  onPin?: (messageId: string) => void;
  onCreatePoll?: (messageId: string) => void;
  onEdit?: (messageId: string, currentBody: string) => void;
  onDelete?: (messageId: string) => void;
  onVotePoll?: (pollId: string, selectedAnswers: number[]) => void;
  onClosePoll?: (pollId: string) => void;
  myPseudonymKey?: string | null;
}

const ThreadPanel: Component<ThreadPanelProps> = (props) => {
  const [body, setBody] = createSignal("");
  let containerRef: HTMLDivElement | undefined;

  createEffect(() => {
    props.messages.length;
    requestAnimationFrame(() => {
      if (containerRef) containerRef.scrollTop = containerRef.scrollHeight;
    });
  });

  function handleSend(): void {
    const text = body().trim();
    if (!text || !props.thread) return;
    props.onSend(props.communityId, props.thread.id, text);
    setBody("");
  }

  function handleKeyDown(e: KeyboardEvent): void {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  function senderName(msg: Message): string {
    return msg.isOwn ? "You" : msg.senderId;
  }

  return (
    <Show when={props.thread}>
      {(thread) => (
        <div class="thread-panel">
          <div class="thread-panel-header">
            <span class="nf-icon">{ICON_THREAD}</span>
            <span class="thread-panel-title">{thread().name}</span>
            <Show when={!thread().archived}>
              <button class="thread-panel-archive-btn" onClick={() => props.onArchive(props.communityId, thread().id)} title="Archive thread">
                <span class="nf-icon">{ICON_ARCHIVE}</span>
              </button>
            </Show>
            <Show when={thread().archived}>
              <button class="thread-panel-archive-btn" onClick={() => props.onUnarchive(props.communityId, thread().id)} title="Unarchive thread">
                <span class="nf-icon">{ICON_ARCHIVE}</span>
              </button>
            </Show>
            <button class="modal-close-btn" onClick={props.onClose}>
              <span class="nf-icon">{ICON_CLOSE}</span>
            </button>
          </div>
          <div class="thread-panel-messages" ref={containerRef}>
            <For each={props.messages}>
              {(msg) => (
                <MessageBubble
                  communityId={props.communityId}
                  message={msg}
                  senderName={senderName(msg)}
                  myPseudonymKey={props.myPseudonymKey}
                  onReply={props.onReply}
                  onReaction={props.onReaction ? (emoji) => props.onReaction!(msg.serverMessageId!, emoji) : undefined}
                  onRemoveReaction={props.onRemoveReaction ? (emoji) => props.onRemoveReaction!(msg.serverMessageId!, emoji) : undefined}
                  onPin={props.onPin}
                  onCreatePoll={props.onCreatePoll}
                  onEdit={props.onEdit}
                  onDelete={props.onDelete}
                  onVotePoll={props.onVotePoll}
                  onClosePoll={props.onClosePoll}
                />
              )}
            </For>
          </div>
          <Show when={!thread().archived}>
            <div class="message-input-wrapper">
              <textarea
                class="message-input message-input-field"
                placeholder="Reply to thread..."
                value={body()}
                onInput={(e) => setBody(e.currentTarget.value)}
                onKeyDown={handleKeyDown}
                rows={2}
              />
            </div>
          </Show>
        </div>
      )}
    </Show>
  );
};

export default ThreadPanel;
