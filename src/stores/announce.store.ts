// Architecture §32 a11y — the announcement state behind `<AnnounceRegion />`.
//
// Two module-level signals (one polite, one assertive) so screen
// readers can pick the right cadence. The text is cleared after a short
// window so the reader doesn't re-announce on tab focus.
//
// This lived inside `components/common/AnnounceRegion.tsx` alongside the
// component that renders it, which made every non-presentation caller —
// `handlers/community/dispatcher_voice.ts` among them — import a
// component to announce a string. Module-level signals are store state;
// the component is the view of them.
//
// Usage:
//   import { announce } from "../stores/announce.store";
//   announce("Notification sound updated");
//   announce("Connection lost", "assertive");

import { createSignal } from "solid-js";

const [politeMessage, setPoliteMessage] = createSignal("");
const [assertiveMessage, setAssertiveMessage] = createSignal("");

const ANNOUNCE_CLEAR_MS = 1500;

let politeClear: ReturnType<typeof setTimeout> | null = null;
let assertiveClear: ReturnType<typeof setTimeout> | null = null;

export { politeMessage, assertiveMessage };

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

/** Cancel both pending clear timers — called from the region's `onCleanup`. */
export function cancelPendingClears(): void {
  if (politeClear) clearTimeout(politeClear);
  if (assertiveClear) clearTimeout(assertiveClear);
}
