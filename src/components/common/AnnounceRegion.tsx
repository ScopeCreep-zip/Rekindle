import { Component, onCleanup } from "solid-js";

import {
  announce,
  assertiveMessage,
  cancelPendingClears,
  politeMessage,
} from "../../stores/announce.store";

/**
 * Architecture §32 a11y — the two live regions screen readers watch.
 *
 * The state and the `announce()` helper live in
 * `stores/announce.store.ts`; this is the view of them. Callers should
 * import `announce` from the store rather than from here — importing a
 * component to announce a string is what put handlers on the wrong side
 * of the presentation boundary.
 *
 * Mount `<AnnounceRegion />` once at the top of each window's render
 * tree (e.g., `BuddyListWindow`, `CommunityWindow`). It does not
 * portal across windows because each Tauri webview has its own DOM.
 */
const AnnounceRegion: Component = () => {
  onCleanup(cancelPendingClears);

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

export { announce };
export default AnnounceRegion;
