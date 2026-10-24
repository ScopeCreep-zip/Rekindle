/**
 * The E2E bridge (`rekindle-e2e-server`) for the real-backend suite.
 *
 * The bridge binds a random loopback port and mints a per-run token,
 * then prints both on one line; `e2e-server-setup.ts` (the e2e project's
 * global setup) starts it, reads that line and hands the address and
 * token to the tests through the environment. Every request must carry
 * the token, so a webpage that finds the port still cannot drive it.
 *
 * - `e2ePost` calls the bridge from the test (Node) side.
 * - `test` gives each page the bridge before any script runs, where
 *   `src/ipc/invoke.ts` reads it (`window.__REKINDLE_E2E__`).
 */

import { test as base } from "@playwright/test";

/** The header the bridge reads the token from. */
export const TOKEN_HEADER = "x-rekindle-e2e-token";

/** The line prefix the bridge prints its address and token after. */
export const READY_PREFIX = "REKINDLE_E2E_READY ";

export interface E2EBridge {
  url: string;
  token: string;
}

/** The running bridge, from the variables global setup exported. */
export function e2eBridge(): E2EBridge {
  const url = process.env.REKINDLE_E2E_URL;
  const token = process.env.REKINDLE_E2E_TOKEN;
  if (!url || !token) {
    throw new Error("the E2E bridge is not running (run with E2E=true)");
  }
  return { url, token };
}

/** POST `body` to the bridge's `path` with the run's token. */
export async function e2ePost(path: string, body?: unknown): Promise<Response> {
  const { url, token } = e2eBridge();
  return fetch(`${url}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", [TOKEN_HEADER]: token },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

/** Playwright's `test`, with every page told where the bridge is. */
export const test = base.extend({
  page: async ({ page }, use) => {
    await page.addInitScript((bridge: E2EBridge) => {
      Object.assign(window, { __REKINDLE_E2E__: bridge });
    }, e2eBridge());
    await use(page);
  },
});

export { expect } from "@playwright/test";
