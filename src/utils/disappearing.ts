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

/** `seconds` as its two largest units, the smaller rounded up: the larger
 *  unit's index, its count, and the next unit's count (0 when none). The
 *  same rule as `rekindle_types::disappearing::parts`, whose tests are its
 *  spec: a timer and a fresh message's countdown read the same ("1 day 6
 *  hours" on the button and "1d 6h" on the bubble). */
function parts(seconds: number): [number, number, number] {
  const found = UNITS.findIndex((u) => seconds >= u.seconds);
  const top = found < 0 ? UNITS.length - 1 : found;
  if (top + 1 >= UNITS.length) return [top, seconds, 0];
  const small = UNITS[top + 1].seconds;
  const rounded = Math.ceil(seconds / small) * small;
  if (top > 0 && rounded >= UNITS[top - 1].seconds) return parts(rounded);
  const unit = UNITS[top].seconds;
  return [top, Math.floor(rounded / unit), (rounded % unit) / small];
}

function words(index: number, count: number): string {
  return `${count} ${count === 1 ? UNITS[index].one : UNITS[index].many}`;
}

function inWords(seconds: number): string {
  const [top, hi, lo] = parts(seconds);
  return lo === 0 ? words(top, hi) : `${words(top, hi)} ${words(top + 1, lo)}`;
}

/** `seconds` in words: "Off", "30 seconds", "1 week", "1 hour 30 minutes",
 *  "1 day 6 hours". */
export function durationText(seconds: number): string {
  return seconds <= 0 ? "Off" : inWords(seconds);
}

/** What is left of `ms`, in its two largest units, the smaller rounded up:
 *  "4w", "1d 6h", "12m", "30s". */
export function remainingShort(ms: number): string {
  const [top, hi, lo] = parts(Math.max(0, Math.ceil(ms / 1000)));
  const short = (index: number, count: number): string => `${count}${UNITS[index].short}`;
  return lo === 0 ? short(top, hi) : `${short(top, hi)} ${short(top + 1, lo)}`;
}

/** The same, in words for a screen reader: "1 day 6 hours". */
export function remainingText(ms: number): string {
  return inWords(Math.max(0, Math.ceil(ms / 1000)));
}

/** A timer change's timeline notice: who set it, to what. */
export function timerNoticeText(who: string, own: boolean, seconds: number): string {
  const subject = own ? "You" : who;
  return seconds > 0
    ? `${subject} set disappearing messages to ${durationText(seconds)}`
    : `${subject} turned off disappearing messages`;
}
