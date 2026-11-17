import { onCleanup } from "solid-js";

/**
 * Plan E7.7.2: `data-window-inactive` on `:root` while this window does not
 * have focus, so the generated region rules
 * (`src/styles/generated/regions.css`, `:where(:root[data-window-inactive])`)
 * show a skin's window-inactive images. It mirrors WebKit's
 * `:window-inactive` pseudo-class, which the scrollbar parts use: the
 * webview's `window` gets `blur` when its OS window loses focus and `focus`
 * when it regains it. Mounted once per webview, in `main.tsx`'s `App`.
 */
export function useWindowInactive(): void {
  const root = document.documentElement;
  const set = (inactive: boolean): void => {
    root.toggleAttribute("data-window-inactive", inactive);
  };
  const onFocus = (): void => set(false);
  const onBlur = (): void => set(true);
  set(!document.hasFocus());
  window.addEventListener("focus", onFocus);
  window.addEventListener("blur", onBlur);
  onCleanup(() => {
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
    root.removeAttribute("data-window-inactive");
  });
}
