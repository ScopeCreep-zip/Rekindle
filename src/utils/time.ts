/** `ms` in its largest whole unit from minutes up ("5m", "3h", "2d", and with `months` "4mo" past 30 days). */
function largestUnit(ms: number, months: boolean): string {
  const minutes = Math.floor(ms / 60000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (!months || days < 30) return `${days}d`;
  return `${Math.floor(days / 30)}mo`;
}

export function formatRelativeTime(timestampMs: number): string {
  const diffMs = Date.now() - timestampMs;
  if (diffMs < 60_000) return "just now";
  return `${largestUnit(diffMs, true)} ago`;
}

export function formatTimeUntil(epochMs: number): string {
  const diffMs = epochMs - Date.now();
  if (diffMs <= 0) return "expired";
  return largestUnit(diffMs, false);
}

/**
 * Countdown to a scheduled event, from a Unix-**seconds** timestamp
 * (e.g. "In 20m", "Started").
 *
 * Distinct from [`formatTimeUntil`], which takes **milliseconds**,
 * returns "expired" rather than "Started", and omits the "In " prefix.
 * The events panel and the community-window sidebar each carried an
 * identical private copy of this — and neither could use
 * `formatTimeUntil`, because despite the near-identical name it speaks
 * a different unit and vocabulary.
 */
export function formatEventCountdown(epochSecs: number): string {
  const now = Math.floor(Date.now() / 1000);
  const diff = epochSecs - now;
  if (diff <= 0) return "Started";
  if (diff < 3600) return `In ${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `In ${Math.floor(diff / 3600)}h`;
  return `In ${Math.floor(diff / 86400)}d`;
}
