/**
 * Conditional IPC invoke wrapper.
 *
 * - Production / Tauri webview: delegates to `@tauri-apps/api/core` invoke (static import)
 * - E2E testing (VITE_E2E=true): sends HTTP POST to the E2E server
 *
 * Window navigation commands (show_buddy_list, etc.) trigger browser
 * navigation in E2E mode since there's no Tauri window manager.
 *
 * `hostInvoke` is the wrapper for Host-call commands (plan E1.1): they
 * answer `Done<T>` or reject with a `CommandError`. A review the Host holds
 * is rendered by the registered intent presenter (the router root's
 * `IntentDialog`) and the call resolves with its commit; an undo offer is
 * shown by the registered undo presenter (a toast with Undo).
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { Done, IntentReview, UndoOffer } from "./commands/dto";
import { isCommandError } from "./commands/dto";

/** The E2E bridge's address and per-run token, which the Playwright
 *  fixture (`e2e/fixtures/e2e-server.ts`) sets before any script runs. */
interface E2EBridge {
  url: string;
  token: string;
}

function e2eBridge(): E2EBridge {
  const bridge = (window as Window & { __REKINDLE_E2E__?: E2EBridge })
    .__REKINDLE_E2E__;
  if (bridge === undefined) {
    throw new Error("VITE_E2E is set but the page was not given the E2E bridge");
  }
  return bridge;
}

/** Map window commands to browser routes for E2E navigation.
 *
 * Only commands where the Rust backend performs window transitions belong here.
 * `logout` is NOT here — `handleLogout()` manages its own navigation with
 * the `?account=` pre-select param.
 */
const E2E_NAVIGATIONS: Record<string, string> = {
  show_buddy_list: "/buddy-list",
};

export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (import.meta.env.VITE_E2E === "true") {
    return invokeViaHttp<T>(cmd, args);
  }

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
export async function hostInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
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

async function invokeViaHttp<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const { url, token } = e2eBridge();
  const res = await fetch(`${url}/invoke`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "x-rekindle-e2e-token": token },
    body: JSON.stringify({ cmd, args: args ?? {} }),
  });

  const body = await res.json();

  if (!res.ok) {
    // The same `CommandError` object the Tauri path rejects with.
    throw body.error ?? {
      type: "failed",
      data: { code: res.status, message: `Command ${cmd} failed`, remediation: null },
    };
  }

  // Navigate for window commands in E2E mode
  if (cmd in E2E_NAVIGATIONS) {
    window.location.href = E2E_NAVIGATIONS[cmd];
  }

  // A Host call answers `{ result, undo }`: the `Done` the Tauri path
  // resolves with.
  return ("undo" in body ? { value: body.result, undo: body.undo } : body.result) as T;
}
