/**
 * Focus return for modal dialogs (wiring guide §13c: "on Esc or close,
 * return focus to the element that opened it").
 *
 * Kobalte's modal `Dialog.Content` traps focus, but on close it focuses
 * only its `Dialog.Trigger`. Our dialogs open from state (a button, a menu
 * item, a backend event), not from a Trigger, so focus fell to `<body>`.
 * These handlers record what had focus when the dialog opened (Kobalte
 * calls `onOpenAutoFocus` before it moves focus in) and put it back on
 * close, if that element is still in the document.
 *
 * Spread the result onto `Dialog.Content`. `onOpen` runs after the opener
 * is recorded, for a dialog that picks its own first focus.
 */
export function returnFocusToOpener(onOpen?: (e: Event) => void): {
  onOpenAutoFocus: (e: Event) => void;
  onCloseAutoFocus: (e: Event) => void;
} {
  let opener: HTMLElement | null = null;
  return {
    onOpenAutoFocus: (e: Event) => {
      const active = document.activeElement;
      opener = active instanceof HTMLElement && active !== document.body ? active : null;
      onOpen?.(e);
    },
    onCloseAutoFocus: (e: Event) => {
      const target = opener;
      opener = null;
      if (target?.isConnected) {
        e.preventDefault();
        target.focus({ preventScroll: true });
      }
    },
  };
}
