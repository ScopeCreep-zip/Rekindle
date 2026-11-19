// Disappearing-message durations as the chat window words them (plan
// E7.1). The values themselves (presets, bound, each message's expiry) are
// the Host's; this only names them.

const UNITS: { seconds: number; one: string; many: string; short: string }[] = [
  { seconds: 7 * 24 * 60 * 60, one: "week", many: "weeks", short: "w" },
  { seconds: 24 * 60 * 60, one: "day", many: "days", short: "d" },
  { seconds: 60 * 60, one: "hour", many: "hours", short: "h" },
  { seconds: 60, one: "minute", many: "minutes", short: "m" },
  { seconds: 1, one: "second", many: "seconds", short: "s" },
];

/** The custom value's units, longest first (Signal Desktop's custom timer
 *  dialog offers the same five). */
export const CUSTOM_UNITS = UNITS.map((u) => ({ seconds: u.seconds, label: u.many }));

/** `seconds` in words: "Off", "30 seconds", "1 week", "90 minutes". */
export function durationText(seconds: number): string {
  if (seconds <= 0) return "Off";
  const unit = UNITS.find((u) => seconds % u.seconds === 0) ?? UNITS[UNITS.length - 1];
  const n = seconds / unit.seconds;
  return `${n} ${n === 1 ? unit.one : unit.many}`;
}

/** What is left of `ms`, rounded up to its largest unit: "4w", "3d",
 *  "5h", "12m", "30s". */
export function remainingShort(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  const unit = UNITS.find((u) => seconds >= u.seconds) ?? UNITS[UNITS.length - 1];
  return `${Math.ceil(seconds / unit.seconds)}${unit.short}`;
}

/** The same, in words for a screen reader: "5 hours". */
export function remainingText(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000));
  const unit = UNITS.find((u) => seconds >= u.seconds) ?? UNITS[UNITS.length - 1];
  const n = Math.ceil(seconds / unit.seconds);
  return `${n} ${n === 1 ? unit.one : unit.many}`;
}

/** A timer change's timeline notice: who set it, to what. */
export function timerNoticeText(who: string, own: boolean, seconds: number): string {
  const subject = own ? "You" : who;
  return seconds > 0
    ? `${subject} set disappearing messages to ${durationText(seconds)}`
    : `${subject} turned off disappearing messages`;
}
