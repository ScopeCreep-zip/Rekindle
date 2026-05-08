import { defineConfig, devices } from "@playwright/test";

// Playwright config for the security suite.
//
// Kept separate from `playwright.config.ts` so this can be run without
// touching the existing mock + e2e projects. Invoke with:
//
//   pnpm exec playwright test --config playwright.security.config.ts
//
// CI: `.github/workflows/lint.yml` runs this in the security job.
//
// All security tests run against the mock-IPC project (no real Rust
// backend) — the suite tests rendering behaviour, CSP, and deep-link
// handling, none of which need a live Veilid node.

export default defineConfig({
  testDir: "./e2e/security",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: process.env.CI ? "github" : "html",
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "security",
      testMatch: "**/*.spec.ts",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    // Run the SolidJS frontend in Playwright-mock mode so security
    // tests don't depend on a Veilid node.
    command: "VITE_PLAYWRIGHT=true pnpm dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
