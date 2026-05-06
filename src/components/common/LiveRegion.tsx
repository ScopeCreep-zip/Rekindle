import { Component, JSX } from "solid-js";

interface LiveRegionProps {
  /**
   * Optional aria-label so screen readers announce a name when the
   * region first appears. Default is "messages" since the most common
   * consumer is the chat / thread message list.
   */
  label?: string;
  /**
   * `polite` (default) waits for the user to pause; `assertive`
   * interrupts. Chat / activity feeds should always stay polite to
   * avoid drowning out other announcements.
   */
  priority?: "polite" | "assertive";
  children: JSX.Element;
}

/**
 * Architecture §32 a11y — `role="log"` wrapper for streaming chat /
 * thread message lists. The container MUST exist in the DOM before the
 * first message is injected; many screen readers ignore late-mounted
 * live regions. Render as a normal flow container (not visually
 * hidden) so the chat scroller IS the live region.
 *
 * `aria-relevant="additions"` keeps deletions silent. `aria-atomic`
 * stays false (default) so only the new message is read, not the
 * entire history on every update.
 */
const LiveRegion: Component<LiveRegionProps> = (props) => {
  return (
    <div
      role="log"
      aria-live={props.priority ?? "polite"}
      aria-relevant="additions"
      aria-atomic="false"
      aria-label={props.label ?? "messages"}
    >
      {props.children}
    </div>
  );
};

export default LiveRegion;
