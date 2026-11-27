import type { ProfileField } from "../ipc/commands/dto";

/**
 * One profile field's edit against the value it had when editing began
 * (plan PF2): the same value is `unchanged`, so a Host change to a field the
 * user did not touch survives the save; no value, or an empty one, is
 * `clear`; anything else is `set`.
 */
function fieldOp<T>(
  next: T | null,
  baseline: T | null,
  isEmpty: (v: T) => boolean,
  same: (a: T | null, b: T | null) => boolean,
): ProfileField<T> {
  if (same(next, baseline)) return { op: "unchanged" };
  if (next === null || isEmpty(next)) return { op: "clear" };
  return { op: "set", value: next };
}

const equal = <T>(a: T, b: T): boolean => a === b;
const never = (): boolean => false;

/** A text field (bio, pronouns, custom status, display name): empty clears. */
export function textField(next: string, baseline: string): ProfileField<string> {
  return fieldOp(next, baseline, (v) => v === "", equal);
}

/** A picture reference: none clears. */
export function refField(next: string | null, baseline: string | null): ProfileField<string> {
  return fieldOp(next, baseline, never, equal);
}

/** The theme colour: none returns the card to the community's default colour. */
export function colourField(next: number | null, baseline: number | null): ProfileField<number> {
  return fieldOp(next, baseline, never, equal);
}

/** The badge list: an empty list clears. */
export function listField(next: string[], baseline: string[]): ProfileField<string[]> {
  return fieldOp(next, baseline, (v) => v.length === 0, (a, b) => a !== null && b !== null && a.length === b.length && a.every((x, i) => x === b[i]));
}
