import { Component, For, Show } from "solid-js";
import { toastState, removeToast } from "../../stores/toast.store";
import { intentState, takeUndo } from "../../stores/intent.store";
import { kindPhrase } from "../../utils/intent_text";

/** Transient messages, announced politely (`role="status"`), and the undo
 *  offer of an action that just ran, with its one action. */
const ToastContainer: Component = () => (
  <div class="toast-container" role="status" aria-live="polite">
    <For each={toastState.toasts}>
      {(toast) => (
        <div
          class={`toast-item toast-${toast.type}`}
          onClick={() => removeToast(toast.id)}
        >
          {toast.message}
        </div>
      )}
    </For>
    <Show when={intentState.undo}>
      {(offer) => (
        <div class="toast-item toast-info toast-undo">
          <span>Done.</span>
          <button
            type="button"
            class="toast-undo-button"
            aria-label={`Undo: ${kindPhrase(offer().inverse)}`}
            onClick={() => void takeUndo()}
          >
            Undo
          </button>
        </div>
      )}
    </Show>
  </div>
);

export default ToastContainer;
