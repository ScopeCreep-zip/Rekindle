/**
 * Conditional IPC invoke wrapper.
 *
 * - Production / Tauri webview: delegates to `@tauri-apps/api/core` invoke (static import)
 * - E2E testing (VITE_E2E=true): sends HTTP POST to the E2E server
 *
 * Window navigation commands (show_buddy_list, etc.) trigger browser
 * navigation in E2E mode since there's no Tauri window manager.
 */

import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const E2E_SERVER = "http://127.0.0.1:3001";

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

async function invokeViaHttp<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const res = await fetch(`${E2E_SERVER}/invoke`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ cmd, args: args ?? {} }),
  });

  const body = await res.json();

  if (!res.ok) {
    throw new Error(body.error || `Command ${cmd} failed`);
  }

  // Navigate for window commands in E2E mode
  if (cmd in E2E_NAVIGATIONS) {
    window.location.href = E2E_NAVIGATIONS[cmd];
  }

  return body.result as T;
}
