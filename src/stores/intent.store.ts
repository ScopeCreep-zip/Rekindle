/**
 * The open intent review (plan E1.1, U1.6; ADR 0019): the action the Host
 * holds for this window, the proof the `IntentDialog` collects, and the
 * undo offer of an action that ran. Frontends render only — the Host's gate
 * decides what is held and what proof it needs.
 *
 * Reviews queue: the dialog shows the first. A review comes from a Host
 * call this window made (`hostInvoke`, which resolves with the commit) or
 * from the backend (`IntentPending`, whose commit's answer is the Host's).
 */

import { createStore } from "solid-js/store";
import {
  hostInvoke,
  invoke,
  registerIntentPresenter,
  registerUndoPresenter,
} from "../ipc/invoke";
import type {
  CommandError, Done, IntentProof, IntentReview, UndoOffer,
} from "../ipc/commands/dto";
import { isCommandError } from "../ipc/commands/dto";
import { errorMessage } from "../utils/error";
import { kindPhrase } from "../utils/intent_text";
import { addToast, removeToast } from "./toast.store";

interface Pending {
  review: IntentReview;
  /** The backend hold this review belongs to (S7d-keys.10 part 2b), or null for a window's own call. */
  holdId: string | null;
  resolve: (done: Done<unknown>) => void;
  reject: (error: unknown) => void;
}

interface IntentState {
  /** The review the dialog shows. */
  review: IntentReview | null;
  /** Why the last commit did not run (a wrong passphrase), shown in the dialog. */
  error: string | null;
  busy: boolean;
  /** The undo offer of the action that just ran. */
  undo: UndoOffer | null;
}

const [intentState, setIntentState] = createStore<IntentState>({
  review: null,
  error: null,
  busy: false,
  undo: null,
});

const queue: Pending[] = [];

function showFirst(): void {
  setIntentState({ review: queue[0]?.review ?? null, error: null, busy: false });
}

/** Hold `review` for the dialog; resolves with its commit's `Done`. */
export function presentReview(review: IntentReview): Promise<Done<unknown>> {
  return new Promise((resolve, reject) => {
    queue.push({ review, holdId: null, resolve, reject });
    if (queue.length === 1) showFirst();
  });
}

/** A review the backend holds, and its hold (plan S7d-keys.10 part 2b). */
export interface HeldReview {
  review: IntentReview;
  holdId: string;
}

/** Queue or refresh one held review in place; whether the queue changed its front. */
function upsertHeld({ review, holdId }: HeldReview): void {
  const held = queue.find((p) => p.holdId === holdId);
  if (held) {
    // The same review again (a snapshot restating it): nothing changes, not
    // the dialog's proof, not an answer in flight.
    if (held.review.intentId === review.intentId) return;
    held.review = review;
    // While its commit or cancel is in flight the dialog stays busy: the
    // answer settles this entry, and a refreshed review comes back with it.
    if (queue[0] === held && !intentState.busy) setIntentState({ review, error: null });
    return;
  }
  queue.push({ review, holdId, resolve: () => {}, reject: () => {} });
  if (queue.length === 1) showFirst();
}

/**
 * A live review the backend holds: one entry per hold, a refresh (a new
 * `intentId`) replacing it in place. The Host resolves the hold, so
 * nothing here awaits it.
 */
export function presentHeldReview(held: HeldReview): void {
  upsertHeld(held);
}

/**
 * The Host's complete set of held reviews (a standing snapshot, sent on
 * every subscribe and resume): afterwards the backend-held entries are
 * exactly `set` (an empty set clears them); a window's own reviews stay.
 */
export function applyHeldReviews(set: HeldReview[]): void {
  const front = queue[0];
  const kept = new Set(set.map((h) => h.holdId));
  for (let i = queue.length - 1; i >= 0; i--) {
    const holdId = queue[i].holdId;
    if (holdId !== null && !kept.has(holdId)) queue.splice(i, 1);
  }
  if (queue[0] !== front) showFirst();
  for (const held of set) upsertHeld(held);
}

/**
 * Remove `entry`, the one an answer belongs to, wherever it now is, and
 * show the next. An entry already gone (the Host's snapshot dropped its
 * hold) changes nothing: its late answer must not reset the review the
 * dialog now shows.
 */
function settle(entry: Pending): void {
  const at = queue.indexOf(entry);
  if (at < 0) return;
  queue.splice(at, 1);
  showFirst();
}

/**
 * Whether an answer still belongs to `entry`: it was sent for `sent`, and the
 * entry may since have taken a refreshed review (a new id) or left the queue.
 * A stale answer only frees the dialog for the review it now shows; it
 * neither settles nor replaces the entry.
 */
function answerIsCurrent(entry: Pending, sent: string): boolean {
  if (queue.includes(entry) && entry.review.intentId === sent) return true;
  if (queue[0] === entry) setIntentState({ busy: false });
  return false;
}

/** Commit the open review with `proof`. A review whose facts changed comes
 *  back fresh and is shown again; a refused proof (401) keeps the dialog open
 *  with the reason; an expired or unknown review rejects the call. */
export async function commitReview(proof: IntentProof): Promise<void> {
  const pending = queue[0];
  if (pending === undefined || intentState.busy) return;
  setIntentState({ busy: true, error: null });
  const sent = pending.review.intentId;
  try {
    const done = await invoke<Done<unknown>>("intent_commit", { intentId: sent, proof });
    if (!answerIsCurrent(pending, sent)) return;
    pending.resolve(done);
    settle(pending);
  } catch (e) {
    if (!answerIsCurrent(pending, sent)) return;
    if (isCommandError(e) && e.type === "intentRequired") {
      pending.review = e.data;
      if (queue[0] === pending) setIntentState({ review: e.data, busy: false, error: null });
      return;
    }
    // Only the gate's wrong-proof answer (401) keeps the review open; an
    // action's own refusal (a join not admitted, 403) closes it and the
    // call rejects with its reason (IJ3).
    if (isCommandError(e) && e.type === "failed" && e.data.code === 401) {
      if (queue[0] === pending) setIntentState({ busy: false, error: errorMessage(e) });
      return;
    }
    pending.reject(e);
    settle(pending);
  }
}

/** Cancel the open review: the Host drops it (running its own cancel
 *  action for a `cancelSends` review) and the call rejects with the Host's
 *  answer, `{ type: "cancelled" }`, or with the error `intent_cancel` met
 *  (the review had expired). */
export async function cancelReview(): Promise<void> {
  const pending = queue[0];
  if (pending === undefined || intentState.busy) return;
  setIntentState({ busy: true, error: null });
  const sent = pending.review.intentId;
  let answer: unknown;
  try {
    answer = (await invoke<Done<CommandError>>("intent_cancel", { intentId: sent })).value;
  } catch (e) {
    answer = e;
  }
  if (!answerIsCurrent(pending, sent)) return;
  pending.reject(answer);
  settle(pending);
}

let undoTimer: ReturnType<typeof setTimeout> | null = null;
let undoToast: number | null = null;

function clearUndo(): void {
  if (undoTimer !== null) clearTimeout(undoTimer);
  if (undoToast !== null) removeToast(undoToast);
  undoTimer = null;
  undoToast = null;
  setIntentState("undo", null);
}

/** Offer an undo, as a toast with one action (plan E7.3), until the Host's
 *  offer expires: the window is the Host's, not a UI timer. */
function presentUndo(offer: UndoOffer): void {
  clearUndo();
  setIntentState("undo", offer);
  undoToast = addToast("Done.", "info", {
    label: "Undo",
    ariaLabel: `Undo: ${kindPhrase(offer.inverse)}`,
    run: takeUndo,
  });
  undoTimer = setTimeout(clearUndo, Math.max(0, offer.expiresAtMs - Date.now()));
}

/** Take back the offered action. The inverse is a request of its own: a
 *  review it needs is shown like any other. */
export async function takeUndo(): Promise<void> {
  const offer = intentState.undo;
  if (offer === null) return;
  clearUndo();
  await hostInvoke("undo", { id: offer.id });
}

registerIntentPresenter(presentReview);
registerUndoPresenter(presentUndo);

export { intentState };
