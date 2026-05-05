import { Accessor, createSignal, onCleanup } from "solid-js";

/**
 * Architecture §32 a11y — JS-side reduced-motion accessor for animations
 * that can't be guarded by CSS alone (e.g., `requestAnimationFrame`-driven
 * Canvas rendering, `transform`-based participant indicators, scroll
 * smoothing). CSS-driven animations should use the `--motion-duration-*`
 * tokens + `@media (prefers-reduced-motion: no-preference)` wrappers in
 * `global.css` / `animations.css` instead — this hook is for the JS path.
 *
 * Returns a reactive `Accessor<boolean>` that flips when the user toggles
 * the OS preference at runtime (macOS Accessibility → Reduce motion,
 * Windows Settings → Ease of access → Display → Show animations).
 */
export function useReducedMotion(): Accessor<boolean> {
  const query = window.matchMedia("(prefers-reduced-motion: reduce)");
  const [matches, setMatches] = createSignal(query.matches);
  const handler = (e: MediaQueryListEvent): void => {
    setMatches(e.matches);
  };
  query.addEventListener("change", handler);
  onCleanup(() => query.removeEventListener("change", handler));
  return matches;
}
