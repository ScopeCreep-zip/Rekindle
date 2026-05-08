import { Component, For, createSignal } from "solid-js";
import { handleSendCallReaction } from "../../handlers/calls.handlers";

// Wave 12 W12.11 — six-emoji reaction tray. Hidden behind a smiley
// toggle so the call panel stays compact when reactions aren't in
// use. Choices mirror Discord's most-common in-call reactions.
const QUICK_EMOJI = ["👍", "❤️", "😂", "🎉", "🤔", "👋"];

const ReactionsTray: Component = () => {
  const [open, setOpen] = createSignal(false);

  return (
    <div class="call-reactions-tray">
      <button
        type="button"
        class="call-control-btn"
        onClick={() => setOpen((v) => !v)}
        aria-pressed={open()}
        aria-label="Send a reaction"
        title="Reactions"
      >
        😀
      </button>
      <div
        class="call-reactions-grid"
        classList={{ "call-reactions-grid-open": open() }}
        role="menu"
      >
        <For each={QUICK_EMOJI}>
          {(emoji) => (
            <button
              type="button"
              class="call-reaction-btn"
              onClick={() => {
                void handleSendCallReaction(emoji);
                setOpen(false);
              }}
              role="menuitem"
              aria-label={`React with ${emoji}`}
            >
              {emoji}
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

export default ReactionsTray;
