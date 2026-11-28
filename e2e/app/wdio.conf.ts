// Plan E7.14: the real-app suite's one-user specs. WebdriverIO's test
// runner drives the desktop app through its embedded, token-guarded
// WebDriver server (feature `webdriver-test`); `RekindleAppLauncher`
// starts one app per worker on its own temp data root (launcher.ts).
// Its specs size windows, so on Linux the app runs on X11 (`display:
// "x11"`, launcher.ts AppCapability: Wayland gives a client no resize);
// specs/native/ checks Wayland's own window behaviour. `pnpm test:app`
// runs this config, then wdio.peers.conf.ts, wdio.native.conf.ts and wdio.budget.conf.ts.
//
//   pnpm test:app                          build, then run every spec
//   REKINDLE_APP_NO_BUILD=1 pnpm test:app  run against the last build
//   pnpm exec wdio run e2e/app/wdio.conf.ts --spec e2e/app/specs/solo/identity.e2e.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/solo/**/*.e2e.ts"],
  capabilities: [{ [APP_CAPABILITY]: { window: "login", display: "x11" } satisfies AppCapability } as WebdriverIO.Capabilities],
};
