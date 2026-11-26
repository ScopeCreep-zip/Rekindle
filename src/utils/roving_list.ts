import { onCleanup } from "solid-js";

interface RovingListOptions {
  orientation?: "vertical" | "horizontal";
  /** Use the selected item as the tab stop, including after external selection. */
  preferSelected?: boolean;
}

/**
 * Roving-tabindex list (plan E7.3 "List keyboard traversal", wiring guide §13f;
 * WAI-ARIA APG "Managing focus within composite widgets").
 *
 * Exactly one row matching `itemSelector` inside the list is in the tab order
 * (`tabindex="0"`): the one last focused, else the first. ArrowUp/ArrowDown move
 * focus between rows, Home/End jump to the ends, and Shift+F10 or the Menu key
 * open the row's context menu by dispatching `contextmenu` at the row, the event
 * the menu's trigger already handles for the mouse. Enter (open) stays with each
 * row, which knows what opening it means.
 */
export function attachRovingList(
  list: HTMLElement,
  itemSelector: string,
  { orientation = "vertical", preferSelected = false }: RovingListOptions = {},
): () => void {
  let current: HTMLElement | null = null;
  const horizontal = orientation === "horizontal";
  const forward = horizontal ? "ArrowRight" : "ArrowDown";
  const backward = horizontal ? "ArrowLeft" : "ArrowUp";

  const items = (): HTMLElement[] =>
    Array.from(list.querySelectorAll<HTMLElement>(itemSelector));

  function sync(): void {
    const rows = items();
    if (current && !rows.includes(current)) current = null;
    const selected = preferSelected ? rows.find((row) => row.getAttribute("aria-selected") === "true") : undefined;
    const active = selected ?? current ?? rows[0] ?? null;
    for (const row of rows) {
      const want = row === active ? "0" : "-1";
      if (row.getAttribute("tabindex") !== want) row.setAttribute("tabindex", want);
    }
  }

  function rowOf(target: EventTarget | null): HTMLElement | null {
    if (!(target instanceof Element)) return null;
    const row = target.closest<HTMLElement>(itemSelector);
    return row && list.contains(row) ? row : null;
  }

  function onFocusIn(e: FocusEvent): void {
    const row = rowOf(e.target);
    if (row) {
      current = row;
      sync();
    }
  }

  function openMenu(row: HTMLElement): void {
    const rect = row.getBoundingClientRect();
    row.dispatchEvent(
      new MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + Math.min(16, rect.width / 2),
        clientY: rect.top + rect.height / 2,
      }),
    );
  }

  function onKeyDown(e: KeyboardEvent): void {
    const row = rowOf(e.target);
    // Keys typed into a control inside a row (a button) still move between rows,
    // but only when focus is on the row itself do they open its menu.
    if (!row) return;
    if ((e.key === "F10" && e.shiftKey) || e.key === "ContextMenu") {
      if (horizontal || e.target !== row) return;
      e.preventDefault();
      openMenu(row);
      return;
    }
    const rows = items();
    const at = rows.indexOf(row);
    let next: HTMLElement | undefined;
    if (e.key === forward) next = rows[at + 1] ?? (horizontal ? rows[0] : undefined);
    else if (e.key === backward) next = rows[at - 1] ?? (horizontal ? rows[rows.length - 1] : undefined);
    else if (e.key === "Home") next = rows[0];
    else if (e.key === "End") next = rows[rows.length - 1];
    else return;
    e.preventDefault();
    if (next) next.focus();
  }

  const observer = new MutationObserver(sync);
  observer.observe(list, {
    childList: true,
    subtree: true,
    ...(preferSelected && { attributeFilter: ["aria-selected"] }),
  });
  list.addEventListener("focusin", onFocusIn);
  list.addEventListener("keydown", onKeyDown);
  sync();

  return () => {
    observer.disconnect();
    list.removeEventListener("focusin", onFocusIn);
    list.removeEventListener("keydown", onKeyDown);
  };
}

/** A `ref` callback that makes the element a roving list for its lifetime. */
export function rovingList(itemSelector: string, options?: RovingListOptions): (el: HTMLElement) => void {
  return (el) => onCleanup(attachRovingList(el, itemSelector, options));
}
