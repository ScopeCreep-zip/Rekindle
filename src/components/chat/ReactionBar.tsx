import { Component, For, Show } from "solid-js";
import type { ReactionGroup } from "../../stores/chat.store";

interface ReactionBarProps {
  reactions: ReactionGroup[];
  myPseudonymKey: string | null;
  onToggle: (emoji: string) => void;
}

const ReactionBar: Component<ReactionBarProps> = (props) => {
  return (
    <Show when={props.reactions && props.reactions.length > 0}>
      <div class="reaction-bar">
        <For each={props.reactions}>
          {(reaction) => {
            const isActive = () => props.myPseudonymKey ? reaction.reactors.includes(props.myPseudonymKey) : false;
            return (
              <button
                class={`reaction-chip ${isActive() ? "reaction-chip-active" : ""}`}
                onClick={() => props.onToggle(reaction.emoji)}
                title={reaction.reactors.join(", ")}
              >
                <span class="reaction-chip-emoji">{reaction.emoji}</span>
                <span class="reaction-chip-count">{reaction.count}</span>
              </button>
            );
          }}
        </For>
      </div>
    </Show>
  );
};

export default ReactionBar;
