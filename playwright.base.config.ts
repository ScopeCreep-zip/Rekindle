import type { PlaywrightTestConfig } from "@playwright/test";

// Settings shared by every Playwright config in the repo.
//
// `playwright.config.ts` (mock + e2e projects) and
// `playwright.security.config.ts` carried byte-identical `use`,
// retry/worker/reporter and Vite `webServer` blocks. They are here so a
// change to CI retry policy or the dev-server port lands in both suites
// at once — the two configs differ only in which tests they select and
// which backing servers they need.

/** Where `pnpm dev` serves the SolidJS frontend. */
export const FRONTEND_URL = "http://localhost:1420";

/** Health endpoint of the E2E Rust bridge (`e2e-server`). */
export const E2E_BACKEND_HEALTH_URL = "http://127.0.0.1:3001/health";

type WebServerConfig = Extract<
  NonNullable<PlaywrightTestConfig["webServer"]>,
  { command: string }
>;

/**
 * Run/report policy plus the shared `use` block.
 *
 * Spread this first, then add `projects` and `webServer` — the two
 * things that genuinely differ per suite.
 */
export const baseConfig = {
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: process.env.CI ? "github" : "html",
  use: {
    baseURL: FRONTEND_URL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
} satisfies PlaywrightTestConfig;

/**
 * The Vite dev server entry.
 *
 * `mode` picks the IPC layer the frontend builds against: `"e2e"` sets
 * `VITE_E2E` so `invoke` speaks HTTP to the Rust bridge, `"mock"` sets
 * `VITE_PLAYWRIGHT` so `mockIPC` answers instead and no backend is
 * needed.
 */
export function viteDevServer(mode: "e2e" | "mock"): WebServerConfig {
  const flag = mode === "e2e" ? "VITE_E2E=true" : "VITE_PLAYWRIGHT=true";
  return {
    command: `${flag} pnpm dev`,
    url: FRONTEND_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  };
}
