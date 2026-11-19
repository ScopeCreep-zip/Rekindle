import { Component, For } from "solid-js";
import { toastState, removeToast, type Toast } from "../../stores/toast.store";

/** One toast. Clicking it dismisses it; its action, if any, runs and dismisses. */
const ToastItem: Component<{ toast: Toast }> = (props) => (
  <div
    class={`toast-item toast-${props.toast.type}`}
    classList={{ "toast-with-action": props.toast.action !== undefined }}
    onClick={() => removeToast(props.toast.id)}
  >
    <span>{props.toast.message}</span>
    {props.toast.action && (
      <button
        type="button"
        class="toast-action-button"
        aria-label={props.toast.action.ariaLabel}
        onClick={(e) => {
          e.stopPropagation();
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
