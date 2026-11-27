import type { ProfileField } from "../ipc/commands/dto";

/**
 * One profile field's edit against the value it had when editing began
 * (plan PF2): the same value is `unchanged`, so a Host change to a field the
 * user did not touch survives the save; an emptied or removed value is
 * `clear`; anything else is `set`.
 */
function fieldOp<T>(next: T, baseline: T, isEmpty: (v: T) => boolean, same: (a: T, b: T) => boolean): ProfileField<T> {
  if (same(next, baseline)) return { op: "unchanged" };
  if (isEmpty(next)) return { op: "clear" };
  return { op: "set", value: next };
}

/** A text field (bio, pronouns, custom status, display name): empty clears. */
export function textField(next: string, baseline: string): ProfileField<string> {
  return fieldOp(next, baseline, (v) => v === "", (a, b) => a === b);
}

/** A picture reference: none clears. */
export function refField(next: string | null, baseline: string | null): ProfileField<string> {
  const op = fieldOp(next, baseline, (v) => v === null, (a, b) => a === b);
  return op.op === "set" && op.value !== null ? { op: "set", value: op.value } : op.op === "set" ? { op: "clear" } : op;
}

/** The badge list: an empty list clears. */
export function listField(next: string[], baseline: string[]): ProfileField<string[]> {
  return fieldOp(next, baseline, (v) => v.length === 0, (a, b) => a.length === b.length && a.every((x, i) => x === b[i]));
}

/** The theme colour: none returns the card to the community's default colour. */
export function colourField(next: number | null, baseline: number | null): ProfileField<number> {
  const op = fieldOp(next, baseline, (v) => v === null, (a, b) => a === b);
  return op.op === "set" && op.value !== null ? { op: "set", value: op.value } : op.op === "set" ? { op: "clear" } : op;
}
