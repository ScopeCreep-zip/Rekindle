/**
 * IPC invoke wrapper over `@tauri-apps/api/core` invoke.
 *
 * `hostInvoke` is the wrapper for Host-call commands (plan E1.1): they
 * answer `Done<T>` or reject with a `CommandError`. A review the Host holds
 * is rendered by the registered intent presenter (the router root's
 * `IntentDialog`) and the call resolves with its commit; an undo offer is
 * shown by the registered undo presenter (a toast with Undo).
 */

import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { Done, IntentReview, UndoOffer } from "./commands/dto";
import { isCommandError } from "./commands/dto";

export { Channel };

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

/** Renders a held action and resolves with its commit's `Done`, or rejects
 *  (cancelled, expired). Registered by `stores/intent.store.ts`. */
type IntentPresenter = (review: IntentReview) => Promise<Done<unknown>>;
/** Shows an undo offer. Registered by `stores/intent.store.ts`. */
type UndoPresenter = (offer: UndoOffer) => void;

let intentPresenter: IntentPresenter | null = null;
let undoPresenter: UndoPresenter | null = null;

export function registerIntentPresenter(presenter: IntentPresenter): void {
  intentPresenter = presenter;
}

export function registerUndoPresenter(presenter: UndoPresenter): void {
  undoPresenter = presenter;
}

/**
 * Call a Host-call command: its value, after the review it needed (if
 * any) was committed. Rejects with the `CommandError` the Host answered
 * (`{ type: "cancelled" }` when the user cancelled the review).
 */
export async function hostInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  let done: Done<T>;
  try {
    done = await invoke<Done<T>>(cmd, args);
  } catch (e) {
    if (!isCommandError(e) || e.type !== "intentRequired") throw e;
    if (intentPresenter === null) {
      throw e;
    }
    done = (await intentPresenter(e.data)) as Done<T>;
  }
  if (done.undo !== null && undoPresenter !== null) {
    undoPresenter(done.undo);
  }
  return done.value;
}
