import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
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
    // Vite dev server — serves the SolidJS frontend
    {
      command: process.env.E2E
        ? "VITE_E2E=true pnpm dev"
        : "VITE_PLAYWRIGHT=true pnpm dev",
      url: "http://localhost:1420",
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
    },
    // E2E backend server — only started for the e2e project
    ...(process.env.E2E
      ? [
          {
            command:
              "cargo run -p rekindle --bin e2e-server --features e2e-server",
            url: "http://127.0.0.1:3001/health",
            reuseExistingServer: !process.env.CI,
            timeout: 120_000,
          },
        ]
      : []),
  ],
});
