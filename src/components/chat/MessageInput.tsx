import { Component, createSignal, createEffect, Show } from "solid-js";
import { handleKeyDown } from "../../handlers/chat.handlers";
import ReplyPreview from "./ReplyPreview";
import { ICON_CLOSE } from "../../icons";

export interface EditMode {
  messageId: string;
  body: string;
}

interface MessageInputProps {
  peerId: string;
  replyTo?: { senderName: string; body: string; messageId?: string } | null;
  editMode?: EditMode | null;
  onSend?: (id: string, body: string, replyToId?: string) => void;
  onDismissReply?: () => void;
  onEditSave?: (messageId: string, newBody: string) => void;
  onEditCancel?: () => void;
  onTyping?: () => void;
  disabled?: boolean;
  disabledMessage?: string;
}

const MessageInput: Component<MessageInputProps> = (props) => {
  const [body, setBody] = createSignal("");

  // When entering edit mode, populate the input with the message body
  createEffect(() => {
    const edit = props.editMode;
    if (edit) {
      setBody(edit.body);
    }
  });

  function getBody(): string {
    return body();
  }

  function clearInput(): void {
    setBody("");
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (props.disabled) return;

    // Edit mode: Escape cancels, Enter saves
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
        }
      }
    } else {
      handleKeyDown(e, props.peerId, getBody, clearInput);
    }
  }

  function onInput(e: InputEvent): void {
    setBody((e.target as HTMLTextAreaElement).value);
    props.onTyping?.();
  }

  return (
    <div class="message-input-wrapper">
      <Show when={props.editMode}>
        <div class="edit-mode-header">
          <span>Editing message</span>
          <button class="edit-mode-cancel" onClick={() => { props.onEditCancel?.(); clearInput(); }} title="Cancel edit (Esc)">
            <span class="nf-icon">{ICON_CLOSE}</span>
          </button>
        </div>
      </Show>
      <Show when={!props.editMode}>
        <ReplyPreview
          replyTo={props.replyTo ?? null}
          onDismiss={() => props.onDismissReply?.()}
        />
      </Show>
      <Show when={props.disabled && !props.editMode}>
        <div class="typing-indicator">
          <span class="typing-label">{props.disabledMessage ?? "You cannot send messages here"}</span>
        </div>
      </Show>
      <Show when={!props.disabled || props.editMode}>
        <textarea
          class={`message-input message-input-field ${props.editMode ? "message-input-editing" : ""}`}
          placeholder={props.editMode ? "Edit your message..." : "Type a message..."}
          value={body()}
          onInput={onInput}
          onKeyDown={onKeyDown}
          rows={2}
        />
      </Show>
    </div>
  );
};

export default MessageInput;
