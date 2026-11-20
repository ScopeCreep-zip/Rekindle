import { createSignal } from "solid-js";

/** A form field over a stored value: see [`useDraft`]. */
export interface Draft {
  /** What the field shows: the user's edit while there is one, else the stored value. */
  value: () => string;
  /** The user typed: from now on the field holds this, whatever the store does. */
  edit: (value: string) => void;
  /** The edit was saved (the store now holds it): the field follows the store again. */
  settle: () => void;
}

/**
 * The dirty-field rule (plan E7.2; walk row ST3): a field shows `stored`
 * until the user edits it, and from the first edit until [`Draft.settle`]
 * it shows only what they typed. A store that fills in later (a window's
 * first `hydrateState`, a reload after `InvalidSession`, a profile event)
 * never overwrites text the user is editing. No effect copies the store
 * into the field, so there is no ordering between the two to lose.
 */
export function useDraft(stored: () => string): Draft {
  const [draft, setDraft] = createSignal<string | null>(null);
  return {
    value: () => draft() ?? stored(),
    edit: (value) => setDraft(value),
    settle: () => setDraft(null),
  };
}
