import { Component, createSignal, onCleanup } from "solid-js";

/**
 * Architecture §32 a11y — module-level `announce(message, priority)`
 * helper for transient status updates that aren't tied to a specific
 * list (toasts, "Saved", "Voice keys rotated", etc.).
 *
 * Two regions are kept mounted (one polite, one assertive) so screen
 * readers can pick the right cadence. The text is cleared after a
 * short window so the reader doesn't re-announce on tab focus.
 *
 * Usage:
 *   import { announce } from "../common/AnnounceRegion";
 *   announce("Notification sound updated");
 *   announce("Connection lost", "assertive");
 *
 * Mount `<AnnounceRegion />` once at the top of each window's render
 * tree (e.g., `BuddyListWindow`, `CommunityWindow`). It does not
 * portal across windows because each Tauri webview has its own DOM.
 */

const [politeMessage, setPoliteMessage] = createSignal("");
const [assertiveMessage, setAssertiveMessage] = createSignal("");

const ANNOUNCE_CLEAR_MS = 1500;

let politeClear: ReturnType<typeof setTimeout> | null = null;
let assertiveClear: ReturnType<typeof setTimeout> | null = null;

export function announce(
  message: string,
  priority: "polite" | "assertive" = "polite",
): void {
  if (!message) return;
  if (priority === "assertive") {
    if (assertiveClear) clearTimeout(assertiveClear);
    setAssertiveMessage("");
    queueMicrotask(() => setAssertiveMessage(message));
    assertiveClear = setTimeout(() => setAssertiveMessage(""), ANNOUNCE_CLEAR_MS);
  } else {
    if (politeClear) clearTimeout(politeClear);
    setPoliteMessage("");
    queueMicrotask(() => setPoliteMessage(message));
    politeClear = setTimeout(() => setPoliteMessage(""), ANNOUNCE_CLEAR_MS);
  }
}

const AnnounceRegion: Component = () => {
  onCleanup(() => {
    if (politeClear) clearTimeout(politeClear);
    if (assertiveClear) clearTimeout(assertiveClear);
  });

  return (
    <>
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        class="live-region-sr-only"
      >
        {politeMessage()}
      </div>
      <div
        role="alert"
        aria-live="assertive"
        aria-atomic="true"
        class="live-region-sr-only"
      >
        {assertiveMessage()}
      </div>
    </>
  );
};

export default AnnounceRegion;
