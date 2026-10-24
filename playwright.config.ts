import { defineConfig, devices } from "@playwright/test";

import { baseConfig, viteDevServer } from "./playwright.base.config";

export default defineConfig({
  ...baseConfig,
  testDir: "./e2e",
  projects: [
    // Frontend-only tests with mocked IPC (fast, no Rust backend needed)
    {
      name: "mock",
      testMatch: ["login.spec.ts", "events.spec.ts", "chat-window.spec.ts", "group-dm.spec.ts"],
      use: { ...devices["Desktop Chrome"] },
    },
    // Real E2E tests against live Rust backend (SQLite + Stronghold + Ed25519)
    {
      name: "e2e",
      testMatch: "auth-e2e.spec.ts",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  // The e2e project's Rust bridge binds a random port with a per-run
  // token; global setup starts it and passes both to the tests
  // (e2e/fixtures/e2e-server.ts).
  ...(process.env.E2E
    ? { globalSetup: "./e2e/fixtures/e2e-server-setup.ts" }
    : {}),
  webServer: [viteDevServer(process.env.E2E ? "e2e" : "mock")],
});
