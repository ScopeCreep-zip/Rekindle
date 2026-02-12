import { Component, Show } from "solid-js";

interface TypingIndicatorProps {
  isTyping: boolean;
  peerName: string;
}

const TypingIndicator: Component<TypingIndicatorProps> = (props) => {
  return (
    <Show when={props.isTyping}>
      <div class="typing-indicator">
        <div class="typing-dots">
          <div class="typing-dot" />
          <div class="typing-dot" />
          <div class="typing-dot" />
        </div>
        <span class="typing-label">{props.peerName} is typing...</span>
      </div>
    </Show>
  );
};

export default TypingIndicator;
