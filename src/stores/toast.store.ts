import { createStore } from "solid-js/store";

/** The one thing a toast offers to do (plan E7.3): a toast that carries one
 *  stays until it is dismissed or acted on (SC 2.2.1), never on a UI timer. */
export interface ToastAction {
  label: string;
  /** The accessible name when `label` alone does not say what it acts on. */
  ariaLabel?: string;
  run: () => void | Promise<void>;
}

export interface Toast {
  id: number;
  message: string;
  type: "success" | "error" | "info";
  action?: ToastAction;
}

interface ToastState {
  toasts: Toast[];
}

let nextId = 1;

const [toastState, setToastState] = createStore<ToastState>({ toasts: [] });

/** Show a toast. One without an action dismisses itself after 4 s; one with an
 *  action stays until dismissed. Returns its id. */
export function addToast(
  message: string,
  type: Toast["type"] = "info",
  action?: ToastAction
): number {
  const id = nextId++;
  setToastState("toasts", (prev) => [...prev, { id, message, type, action }]);
  if (action === undefined) setTimeout(() => removeToast(id), 4000);
  return id;
}

export function removeToast(id: number): void {
  setToastState("toasts", (prev) => prev.filter((t) => t.id !== id));
}

export { toastState };
