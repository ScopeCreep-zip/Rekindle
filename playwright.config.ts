import { defineConfig, devices } from "@playwright/test";

import { baseConfig, viteDevServer } from "./playwright.base.config";

export default defineConfig({
  ...baseConfig,
  testDir: "./e2e",
  projects: [
    // Frontend-only tests with mocked IPC (fast, no Rust backend needed)
    {
      name: "mock",
      testMatch: ["login.spec.ts", "events.spec.ts", "chat-window.spec.ts", "group-dm.spec.ts", "community-removal.spec.ts", "window-focus.spec.ts", "skin-apply.spec.ts", "skin-appearance.spec.ts", "member-list.spec.ts", "profile-window.spec.ts", "channel-history.spec.ts", "channel-awaiting-key.spec.ts", "friend-groups.spec.ts", "status-picker.spec.ts", "community-sidebar.spec.ts", "settings-quiet-hours.spec.ts", "channel-edit.spec.ts", "settings-loaded-values.spec.ts", "community-unheld.spec.ts", "thread-panel.spec.ts", "create-community.spec.ts", "channel-create.spec.ts"],
      use: { ...devices["Desktop Chrome"] },
    },
    // Geometry of every routed window (plan E7.12), mocked IPC like `mock`,
    // on WebKit: the app ships in WKWebView and WebKitGTK, and layout is
    // engine-specific.
    {
      name: "layout",
      testMatch: "layout/**/*.spec.ts",
      use: { ...devices["Desktop Safari"] },
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
