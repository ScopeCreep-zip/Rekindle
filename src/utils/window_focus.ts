/**
 * Whether this window has the focus, on its document root
 * (`<html data-window-focus="active|inactive">`), for presentation that
 * follows it: the theme's inactive text selection (plan ST1,
 * `:root[data-window-focus="inactive"] ::selection`), which a
 * `::selection:window-inactive` rule cannot carry (the CSS build drops a
 * pseudo-class after a pseudo-element). Window focus is the webview's own
 * presentation signal, as `:window-inactive` itself is: no Host state.
 */
let tracking = false;

export function trackWindowFocus(): void {
  if (tracking) return;
  tracking = true;
  const root = document.documentElement;
  const set = (): void => {
    root.dataset.windowFocus = document.hasFocus() ? "active" : "inactive";
  };
  set();
  window.addEventListener("focus", set);
  window.addEventListener("blur", set);
}
