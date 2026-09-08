import { Component, For, createSignal, createEffect, Show } from "solid-js";
import { Popover } from "@kobalte/core/popover";
import { handleKeyDown } from "../../handlers/chat.handlers";
import { handleUploadAttachment } from "../../handlers/community.handlers";
import EmojiPicker from "./EmojiPicker";
import ReplyPreview from "./ReplyPreview";
import { voiceState } from "../../stores/voice.store";
import { ICON_CLOSE, ICON_EMOTICON, ICON_PAPERCLIP } from "../../icons";
import { useMentionAutocomplete } from "./message_input/useMentionAutocomplete";
import { useVoiceRecorder } from "./message_input/useVoiceRecorder";
import { useSlowmode } from "./message_input/useSlowmode";
import type { MessageInputProps } from "./message_input/types";

const ICON_MIC = "\u{F036C}"; // nf-md-microphone
const ICON_MIC_OFF = "\u{F036D}"; // nf-md-microphone_off

// Declared in ./message_input/types.ts and re-exported here so the
// existing import sites keep working. `useSlowmode` needs the props
// type and this file imports `useSlowmode` — declaring it here closed
// a module cycle.
export type { EditMode, MessageInputProps } from "./message_input/types";

const MessageInput: Component<MessageInputProps> = (props) => {
  const [body, setBody] = createSignal("");
  const [showEmojiPicker, setShowEmojiPicker] = createSignal(false);
  let textareaRef: HTMLTextAreaElement | undefined;

  const mentions = useMentionAutocomplete({
    communityId: () => props.communityId,
    body,
    setBody,
    getTextarea: () => textareaRef,
  });
  const recorder = useVoiceRecorder({
    communityId: () => props.communityId,
    channelId: () => props.peerId,
  });
  const slowmode = useSlowmode(props);

  // When entering edit mode, populate the input with the message body.
  createEffect(() => {
    const edit = props.editMode;
    if (edit) setBody(edit.body);
  });

  function getBody(): string {
    return body();
  }

  function clearInput(): void {
    setBody("");
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (slowmode.effectiveDisabled()) return;

    // Mention picker absorbs ArrowUp/Down/Enter/Tab/Escape when open.
    if (mentions.tryConsumeKeyDown(e)) return;

    // Edit mode: Escape cancels, Enter saves.
    if (props.editMode) {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onEditCancel?.();
        clearInput();
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const text = getBody().trim();
        if (text && text !== props.editMode.body) {
          props.onEditSave?.(props.editMode.messageId, text);
        } else {
          props.onEditCancel?.();
        }
        clearInput();
        return;
      }
      return;
    }

    if (props.onSend) {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const text = getBody();
        if (text.trim()) {
          props.onSend(props.peerId, text, props.replyTo?.messageId);
          clearInput();
          props.onDismissReply?.();
          slowmode.recordSent();
        }
      }
    } else {
      handleKeyDown(e, props.peerId, getBody, clearInput);
    }
  }

  function onInput(e: InputEvent): void {
    const target = e.target as HTMLTextAreaElement;
    setBody(target.value);
    // Recompute mention picker query from the new caret position.
    mentions.recomputeOnInput(target.value, target.selectionStart ?? target.value.length);
    props.onTyping?.();
  }

  function insertEmoji(value: string): void {
    setBody((current) => `${current}${value}`);
    setShowEmojiPicker(false);
  }

  async function handleAttachClick(): Promise<void> {
    if (!props.communityId) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: false, directory: false });
    if (!picked) return;
    await handleUploadAttachment(props.communityId, props.peerId, picked as string);
  }

  return (
    <div class="message-input-wrapper">
      <Show when={props.editMode}>
        <div class="edit-mode-header">
          <span>Editing message</span>
          <button
            class="edit-mode-cancel"
            onClick={() => { props.onEditCancel?.(); clearInput(); }}
            title="Cancel edit (Esc)"
            aria-label="Cancel edit"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
          </button>
        </div>
      </Show>
      <Show when={!props.editMode}>
        <ReplyPreview
          replyTo={props.replyTo ?? null}
          onDismiss={() => props.onDismissReply?.()}
        />
      </Show>
      <Show when={slowmode.effectiveDisabled() && !props.editMode}>
        <div class="typing-indicator">
          <span class="typing-label">{slowmode.effectiveDisabledMessage()}</span>
        </div>
      </Show>
      <Show when={!slowmode.effectiveDisabled() || props.editMode}>
        <div class="message-input-shell">
          <Show when={props.communityId && !props.editMode}>
            <button
              class="message-input-attach-btn"
              type="button"
              title="Attach file"
              aria-label="Attach file"
              onClick={() => void handleAttachClick()}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_PAPERCLIP}</span>
            </button>
          </Show>
          <Show when={props.communityId && !props.editMode}>
            <button
              class={`message-input-mic-btn ${recorder.recording() ? "message-input-mic-btn-active" : ""}`}
              type="button"
              title={recorder.recording() ? "Release to send (mousedown to record)" : "Hold to record voice"}
              aria-label={recorder.recording() ? "Recording voice — release to send" : "Hold to record a voice message"}
              aria-pressed={recorder.recording()}
              onMouseDown={() => void recorder.startRecording()}
              onMouseUp={() => void recorder.stopRecording(true)}
              onMouseLeave={() => recorder.recording() && void recorder.stopRecording(true)}
              onTouchStart={() => void recorder.startRecording()}
              onTouchEnd={() => void recorder.stopRecording(true)}
            >
              <span class="nf-icon" aria-hidden="true">{recorder.recording() ? ICON_MIC_OFF : ICON_MIC}</span>
              <Show when={recorder.recording()}>
                <span class="message-input-mic-timer">
                  {Math.floor(recorder.recordedMs() / 1000)}s
                </span>
              </Show>
            </button>
          </Show>
          {/* Plan §Failure 3 — Kobalte Popover with Popover.Portal so the
           * picker isn't clipped by `.message-input-area`'s flex/overflow
           * box. Same pattern as MessageBubble's react picker. */}
          <Popover open={showEmojiPicker()} onOpenChange={setShowEmojiPicker}>
            <Popover.Trigger
              as="button"
              class="message-input-emoji-btn"
              type="button"
              title="Insert emoji"
              aria-label="Insert emoji"
              aria-pressed={showEmojiPicker()}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_EMOTICON}</span>
            </Popover.Trigger>
            <Popover.Portal>
              <Popover.Content class="emoji-picker-popover">
                <EmojiPicker
                  communityId={props.communityId}
                  mode="message"
                  activeVoiceChannelId={voiceState.activeCallType === "community" ? voiceState.channelId : null}
                  onSelect={insertEmoji}
                  onClose={() => setShowEmojiPicker(false)}
                />
              </Popover.Content>
            </Popover.Portal>
          </Popover>
          <textarea
            ref={(el) => (textareaRef = el)}
            class={`message-input message-input-field ${props.editMode ? "message-input-editing" : ""}`}
            placeholder={props.editMode ? "Edit your message..." : "Type a message..."}
            value={body()}
            onInput={onInput}
            onKeyDown={onKeyDown}
            rows={2}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={mentions.mentionQuery() !== null && mentions.mentionCandidates().length > 0}
            aria-controls="mention-popover-list"
            aria-activedescendant={
              mentions.mentionQuery() !== null && mentions.mentionCandidates().length > 0
                ? `mention-row-${mentions.mentionSelected()}`
                : undefined
            }
          />
          {/* Architecture §28.5 — mention autocomplete popover.
           * Implements the ARIA combobox listbox pattern (2026-canon):
           * arrow keys move `mentionSelected`, Enter applies, Esc
           * dismisses; the textarea uses `aria-activedescendant` to
           * point at the focused row without moving DOM focus. Each row is
           * a `<li role="option">` carrying that id; no nested button —
           * the textarea owns focus and mouse interactions live on the
           * option element itself so the focus target and id-referenced
           * node are the same. */}
          <Show when={mentions.mentionQuery() !== null && mentions.mentionCandidates().length > 0}>
            <ul class="mention-popover" role="listbox" id="mention-popover-list">
              <For each={mentions.mentionCandidates()}>
                {(candidate, index) => (
                  <li
                    role="option"
                    id={`mention-row-${index()}`}
                    aria-selected={index() === mentions.mentionSelected()}
                    class={`mention-popover-row ${index() === mentions.mentionSelected() ? "mention-popover-row-active" : ""}`}
                    onMouseEnter={() => mentions.setMentionSelected(index())}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      mentions.applyMention(candidate);
                    }}
                  >
                    <span class={`mention-popover-tag mention-popover-tag-${candidate.kind}`}>{candidate.kind}</span>
                    <span class="mention-popover-label">{candidate.label}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default MessageInput;
