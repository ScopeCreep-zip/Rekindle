// Plan E7.14: the real-app suite. WebdriverIO's test runner drives the
// desktop app through its embedded, token-guarded WebDriver server
// (feature `webdriver-test`); `RekindleAppLauncher` starts one app per
// worker on its own temp data root (launcher.ts).
//
//   pnpm test:app                       build, then run every spec
//   REKINDLE_APP_NO_BUILD=1 pnpm test:app   run against the last build
//   pnpm test:app --spec e2e/app/specs/identity.e2e.ts

import { APP_CAPABILITY, type AppCapability, RekindleAppLauncher } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./specs/**/*.e2e.ts"],
  // One real app at a time: the walk's rule for a shared machine, and a
  // Veilid node per app.
  maxInstances: 1,
  capabilities: [{ [APP_CAPABILITY]: { window: "login" } satisfies AppCapability } as WebdriverIO.Capabilities],
  services: [[RekindleAppLauncher, {}]],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 300_000 },
  reporters: ["spec"],
  // `warn`: the default `info` logs every command's body (typed passphrases
  // included); request headers are never logged at any level.
  logLevel: "warn",
  waitforTimeout: 10_000,
  connectionRetryCount: 1,
};
