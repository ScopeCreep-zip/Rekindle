import { defineConfig, devices } from "@playwright/test";

import {
  E2E_BACKEND_HEALTH_URL,
  baseConfig,
  viteDevServer,
} from "./playwright.base.config";

export default defineConfig({
  ...baseConfig,
  testDir: "./e2e",
  projects: [
    // Frontend-only tests with mocked IPC (fast, no Rust backend needed)
    {
      name: "mock",
      testMatch: "login.spec.ts",
      use: { ...devices["Desktop Chrome"] },
    },
    // Real E2E tests against live Rust backend (SQLite + Stronghold + Ed25519)
    {
      name: "e2e",
      testMatch: "auth-e2e.spec.ts",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: [
    viteDevServer(process.env.E2E ? "e2e" : "mock"),
    // E2E backend server — only started for the e2e project
    ...(process.env.E2E
      ? [
          {
            command:
              "cargo run -p rekindle --bin e2e-server --features e2e-server",
            url: E2E_BACKEND_HEALTH_URL,
            reuseExistingServer: !process.env.CI,
            timeout: 120_000,
          },
        ]
      : []),
  ],
});
