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
  cancelledError,
  hostInvoke,
  invoke,
  registerIntentPresenter,
  registerUndoPresenter,
} from "../ipc/invoke";
import type { Done, IntentProof, IntentReview, UndoOffer } from "../ipc/commands/dto";
import { isCommandError } from "../ipc/commands/dto";
import { errorMessage } from "../utils/error";

interface Pending {
  review: IntentReview;
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
    queue.push({ review, resolve, reject });
    if (queue.length === 1) showFirst();
  });
}

function settle(): void {
  queue.shift();
  showFirst();
}

/** Commit the open review with `proof`. A review whose facts changed comes
 *  back fresh and is shown again; a refused proof keeps the dialog open
 *  with the reason; an expired or unknown review rejects the call. */
export async function commitReview(proof: IntentProof): Promise<void> {
  const pending = queue[0];
  if (pending === undefined || intentState.busy) return;
  setIntentState({ busy: true, error: null });
  try {
    const done = await invoke<Done<unknown>>("intent_commit", {
      intentId: pending.review.intentId,
      proof,
    });
    pending.resolve(done);
    settle();
  } catch (e) {
    if (isCommandError(e) && e.type === "intentRequired") {
      pending.review = e.data;
      setIntentState({ review: e.data, busy: false, error: null });
      return;
    }
    if (isCommandError(e) && e.type === "failed" && e.data.code === 403) {
      setIntentState({ busy: false, error: errorMessage(e) });
      return;
    }
    pending.reject(e);
    settle();
  }
}

/** Cancel the open review: the Host drops it and the call is cancelled. */
export async function cancelReview(): Promise<void> {
  const pending = queue[0];
  if (pending === undefined) return;
  try {
    await invoke("intent_cancel", { intentId: pending.review.intentId });
  } catch {
    // Already expired or committed elsewhere: the call is cancelled either way.
  }
  pending.reject(cancelledError());
  settle();
}

let undoTimer: ReturnType<typeof setTimeout> | null = null;

/** Offer an undo until it expires. */
function presentUndo(offer: UndoOffer): void {
  if (undoTimer !== null) clearTimeout(undoTimer);
  setIntentState("undo", offer);
  undoTimer = setTimeout(
    () => setIntentState("undo", null),
    Math.max(0, offer.expiresAtMs - Date.now()),
  );
}

/** Take back the offered action. The inverse is a request of its own: a
 *  review it needs is shown like any other. */
export async function takeUndo(): Promise<void> {
  const offer = intentState.undo;
  if (offer === null) return;
  setIntentState("undo", null);
  await hostInvoke("undo", { id: offer.id });
}

registerIntentPresenter(presentReview);
registerUndoPresenter(presentUndo);

export { intentState };
