import { createStore } from "solid-js/store";

export interface Toast {
  id: number;
  message: string;
  type: "success" | "error" | "info";
}

interface ToastState {
  toasts: Toast[];
}

let nextId = 1;

const [toastState, setToastState] = createStore<ToastState>({ toasts: [] });

export function addToast(message: string, type: Toast["type"] = "info"): void {
  const id = nextId++;
  setToastState("toasts", (prev) => [...prev, { id, message, type }]);
  setTimeout(() => removeToast(id), 4000);
}

export function removeToast(id: number): void {
  setToastState("toasts", (prev) => prev.filter((t) => t.id !== id));
}

export { toastState };
