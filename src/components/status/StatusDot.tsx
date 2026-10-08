import { Component, Match, Switch } from "solid-js";
import type { UserStatus } from "../../stores/auth.store";

interface StatusDotProps {
  status: UserStatus | string;
}

const statusClassMap: Record<string, string> = {
  online: "status-dot status-dot-online",
  away: "status-dot status-dot-away",
  busy: "status-dot status-dot-busy",
  ingame: "status-dot status-dot-ingame",
  offline: "status-dot status-dot-offline",
};

const statusLabels: Record<string, string> = {
  online: "Online",
  away: "Away",
  busy: "Do not disturb",
  ingame: "In game",
  offline: "Offline",
};

/**
 * Architecture §32 a11y — status indicator with shape redundancy
 * (WCAG 1.4.1 — never rely on color alone). Online + busy + in-game
 * are filled circles with distinct overlay glyphs (none / minus bar /
 * triangle); away is a filled circle with a crescent cutout; offline
 * is an open ring with no fill. Each variant is announced by screen
 * readers via `role="img"` + `aria-label`. The same color tokens
 * carry over from the previous color-only implementation so the
 * dense Xfire visual identity is unchanged for sighted users.
 */
const StatusDot: Component<StatusDotProps> = (props) => {
  const status = (): string => props.status as string;
  const cls = (): string => statusClassMap[status()] ?? "status-dot status-dot-offline";
  const label = (): string => `Status: ${statusLabels[status()] ?? "Offline"}`;

  return (
    <span class={cls()} role="img" aria-label={label()}>
      <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
        <Switch fallback={<circle cx="6" cy="6" r="4.5" fill="currentColor" stroke="none" />}>
          <Match when={status() === "offline"}>
            {/* Open ring — no fill, distinguishable in grayscale. */}
            <circle cx="6" cy="6" r="4" fill="none" stroke="currentColor" stroke-width="1.5" />
          </Match>
          <Match when={status() === "away"}>
            {/* Filled circle with a crescent cutout (clock/moon glyph). */}
            <circle cx="6" cy="6" r="4.5" fill="currentColor" />
            <circle cx="8" cy="5" r="3" fill="var(--color-xfire-bg-dark)" />
          </Match>
          <Match when={status() === "busy"}>
            {/* Filled circle with a horizontal "do not enter" bar. */}
            <circle cx="6" cy="6" r="4.5" fill="currentColor" />
            <rect x="2.5" y="5.25" width="7" height="1.5" fill="var(--color-xfire-bg-dark)" rx="0.5" />
          </Match>
          <Match when={status() === "ingame"}>
            {/* Filled circle with a play-triangle glyph. */}
            <circle cx="6" cy="6" r="4.5" fill="currentColor" />
            <polygon points="4.5,3.5 4.5,8.5 9,6" fill="var(--color-xfire-bg-dark)" />
          </Match>
        </Switch>
      </svg>
    </span>
  );
};

export default StatusDot;
