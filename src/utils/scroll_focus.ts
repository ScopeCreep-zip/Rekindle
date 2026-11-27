import { createEffect, createSignal, onCleanup } from "solid-js";

/** Keep an overflowing reading region in the keyboard order. A short region
 * can still be focused directly without adding a Tab stop. */
export function createScrollFocus<E extends HTMLElement>() {
  const [element, setElement] = createSignal<E>();
  const [overflows, setOverflows] = createSignal(false);

  createEffect(() => {
    const node = element();
    if (!node) return;

    let frame: number | undefined;
    const measure = () => {
      frame = undefined;
      setOverflows(node.scrollHeight > node.clientHeight || node.scrollWidth > node.clientWidth);
    };
    const schedule = () => {
      frame ??= requestAnimationFrame(measure);
    };
    const resize = new ResizeObserver(schedule);
    const content = new MutationObserver(schedule);
    resize.observe(node);
    content.observe(node, { childList: true, subtree: true, characterData: true, attributes: true });
    node.addEventListener("load", schedule, true);
    schedule();

    onCleanup(() => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      node.removeEventListener("load", schedule, true);
      content.disconnect();
      resize.disconnect();
    });
  });

  return { setElement, tabIndex: () => (overflows() ? 0 : -1) };
}
