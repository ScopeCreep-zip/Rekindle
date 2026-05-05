import { Component, For, Show } from "solid-js";
import type { ReactionGroup } from "../../stores/chat.store";
import { communityState } from "../../stores/community.store";

interface ReactionBarProps {
  communityId?: string;
  reactions: ReactionGroup[];
  myPseudonymKey: string | null;
  onToggle: (emoji: string) => void;
}

const ReactionBar: Component<ReactionBarProps> = (props) => {
  function resolveCustomEmojiSrc(expression: string): string | null {
    if (!props.communityId || !expression.startsWith("custom:")) return null;
    const expressionId = expression.slice("custom:".length);
    const community = communityState.communities[props.communityId];
    const match = community?.expressions.find((item) => item.id === expressionId);
    return match?.inlineDataUrl ?? null;
  }

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
                aria-label={`${isActive() ? "Remove" : "Add"} reaction ${reaction.emoji} (${reaction.count} ${reaction.count === 1 ? "person" : "people"})`}
                aria-pressed={isActive()}
              >
                <Show
                  when={resolveCustomEmojiSrc(reaction.emoji)}
                  fallback={<span class="reaction-chip-emoji" aria-hidden="true">{reaction.emoji}</span>}
                >
                  {(src) => <img class="reaction-chip-expression" src={src()} alt="" />}
                </Show>
                <span class="reaction-chip-count" aria-hidden="true">{reaction.count}</span>
              </button>
            );
          }}
        </For>
      </div>
    </Show>
  );
};

export default ReactionBar;
