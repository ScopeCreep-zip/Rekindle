import { Component, For } from "solid-js";
import { callsState } from "../../stores/calls.store";
import { removeCallReaction } from "../../handlers/calls.handlers";

// Wave 12 W12.11 — overlay that floats incoming + outgoing in-call
// reactions over the panel. Each glyph animates upward over ~2 s and
// removes itself from the store on animation end so the list stays
// bounded.
const ReactionFloater: Component = () => {
  return (
    <div class="call-reaction-floater" aria-hidden="true">
      <For each={callsState.recentReactions}>
        {(r) => (
          <span
            class="call-reaction-float"
            onAnimationEnd={() => removeCallReaction(r.id)}
          >
            {r.emoji}
          </span>
        )}
      </For>
    </div>
  );
};

export default ReactionFloater;
