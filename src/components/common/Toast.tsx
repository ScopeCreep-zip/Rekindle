import { type Component, For } from "solid-js";
import { toastState, removeToast, type Toast } from "../../stores/toast.store";

/** One toast. Its message dismisses it; its action, if any, runs and dismisses. */
const ToastItem: Component<{ toast: Toast }> = (props) => (
  <div
    class={`toast-item toast-${props.toast.type}`}
    classList={{ "toast-with-action": props.toast.action !== undefined }}
  >
    <button
      type="button"
      class="toast-message"
      aria-description="Dismiss"
      onClick={() => removeToast(props.toast.id)}
    >
      {props.toast.message}
    </button>
    {props.toast.action && (
      <button
        type="button"
        class="toast-action-button"
        aria-label={props.toast.action.ariaLabel}
        onClick={() => {
          const action = props.toast.action;
          removeToast(props.toast.id);
          void action?.run();
        }}
      >
        {props.toast.action.label}
      </button>
    )}
  </div>
);

/** Plan E7.3 "Toast live regions": both regions are in the page from mount, so
 *  a toast added later is announced. Errors go to the `alert` region
 *  (assertive), every other toast to the `status` region (polite). */
const ToastContainer: Component = () => (
  <div class="toast-container">
    <div class="toast-region" role="status" aria-live="polite">
      <For each={toastState.toasts.filter((t) => t.type !== "error")}>
        {(toast) => <ToastItem toast={toast} />}
      </For>
    </div>
    <div class="toast-region" role="alert" aria-live="assertive">
      <For each={toastState.toasts.filter((t) => t.type === "error")}>
        {(toast) => <ToastItem toast={toast} />}
      </For>
    </div>
  </div>
);

export default ToastContainer;
