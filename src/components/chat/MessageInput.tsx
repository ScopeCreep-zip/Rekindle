import { Component, createSignal } from "solid-js";
import { handleKeyDown } from "../../handlers/chat.handlers";

interface MessageInputProps {
  peerId: string;
  onSend?: (id: string, body: string) => void;
}

const MessageInput: Component<MessageInputProps> = (props) => {
  const [body, setBody] = createSignal("");

  function getBody(): string {
    return body();
  }

  function clearInput(): void {
    setBody("");
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (props.onSend) {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const text = getBody();
        if (text.trim()) {
          props.onSend(props.peerId, text);
          clearInput();
        }
      }
    } else {
      handleKeyDown(e, props.peerId, getBody, clearInput);
    }
  }

  function onInput(e: InputEvent): void {
    setBody((e.target as HTMLTextAreaElement).value);
  }

  return (
    <div class="message-input-wrapper">
      <textarea
        class="message-input message-input-field"
        placeholder="Type a message..."
        value={body()}
        onInput={onInput}
        onKeyDown={onKeyDown}
        rows={2}
      />
    </div>
  );
};

export default MessageInput;
