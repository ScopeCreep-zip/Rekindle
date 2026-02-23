import { createSignal, type Accessor } from "solid-js";

export interface ContextMenuPosition<T> {
  x: number;
  y: number;
  data: T;
}

export interface ContextMenuPrimitive<T> {
  state: Accessor<ContextMenuPosition<T> | null>;
  open: (e: MouseEvent, data: T) => void;
  close: () => void;
}

export function createContextMenu<T>(): ContextMenuPrimitive<T> {
  const [state, setState] = createSignal<ContextMenuPosition<T> | null>(null);

  function open(e: MouseEvent, data: T): void {
    e.preventDefault();
    setState({ x: e.clientX, y: e.clientY, data } as ContextMenuPosition<T>);
  }

  function close(): void {
    setState(null);
  }

  return { state, open, close };
}
