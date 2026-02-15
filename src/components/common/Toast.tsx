import { Component, For } from "solid-js";
import { toastState, removeToast } from "../../stores/toast.store";

const ToastContainer: Component = () => (
  <div class="toast-container">
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
  </div>
);

export default ToastContainer;
