import { defineConfig, devices } from "@playwright/test";

import { baseConfig, viteDevServer } from "./playwright.base.config";

// Playwright config for the security suite.
//
// Kept separate from `playwright.config.ts` so this can be run without
// touching the existing mock + e2e projects. Invoke with:
//
//   pnpm exec playwright test --config playwright.security.config.ts
//
// CI: `.github/workflows/lint.yml` runs this in the `security` job via
// `pnpm test:security`.
//
// All security tests run against the mock-IPC project (no real Rust
// backend) — the suite tests rendering behaviour, CSP, and deep-link
// handling, none of which need a live Veilid node.

export default defineConfig({
  ...baseConfig,
  testDir: "./e2e/security",
  projects: [
    {
      name: "security",
      testMatch: "**/*.spec.ts",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: viteDevServer("mock"),
});
