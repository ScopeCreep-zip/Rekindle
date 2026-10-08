/** Format a millisecond timestamp as a short time string (e.g. "2:45 PM"). */
export function formatTimestamp(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Format a Unix-seconds timestamp as a locale date string (e.g. "1/15/2025"). */
export function formatDateFromSecs(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString();
}

/** Truncate a key to first 8 + ellipsis + last 8 chars (e.g. "abc12345\u2026xyz12345"). */
export function truncateKey(key: string): string {
  if (key.length > 16) return key.slice(0, 8) + "\u2026" + key.slice(-8);
  return key;
}

/** Format Unix seconds as full locale datetime (e.g. "1/15/2025, 2:45:30 PM"). */
export function formatDateTimeSecs(ts: number): string {
  return new Date(ts * 1000).toLocaleString();
}

/** Format a snake_case audit action as readable text (e.g. "create channel"). */
export function formatAction(action: string): string {
  return action.replace(/_/g, " ");
}

/**
 * Format a millisecond timestamp as a relative time string (e.g. "3m ago").
 *
 * Re-exported from `time.ts`, which owns the implementation. This file
 * had its own copy that stopped at days where the other adds a months
 * tier, so the same 60-day-old item read "60d ago" or "2mo ago" purely
 * by which util a component happened to import.
 */
export { formatRelativeTime } from "./time";

/** Format a Unix-seconds expiration timestamp as a relative duration (e.g. "3h", "2d"). */
export function formatExpiry(expiresAt: number | null): string {
  if (expiresAt === null) return "Never";
  const now = Math.floor(Date.now() / 1000);
  if (expiresAt <= now) return "Expired";
  const diff = expiresAt - now;
  if (diff < 3600) return `${Math.ceil(diff / 60)}m`;
  if (diff < 86400) return `${Math.ceil(diff / 3600)}h`;
  return `${Math.ceil(diff / 86400)}d`;
}

/**
 * Format a duration in seconds as "1h 20m" (or "20m" under an hour).
 *
 * `BuddyItem` and `ProfileWindow` each had this as `formatElapsed`,
 * identical but for ProfileWindow prefixing "Playing for ". The prefix
 * is the caller's business; the arithmetic is not.
 */
export function formatDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  return hours > 0 ? `${hours}h ${mins}m` : `${mins}m`;
}
