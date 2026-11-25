// AA1: the real-app spec of auto-away. The app is a debug WebDriver build
// launched with REKINDLE_WDIO_IDLE_SECONDS set past the auto-away threshold
// (the Host's webdriver-only idle override), so auto-away fires at unlock
// on any machine. Its own config, since every other config launches with
// the idle override at 0 (launcher.ts) so no spec races auto-away.
//
//   REKINDLE_APP_NO_BUILD=1 pnpm exec wdio run e2e/app/wdio.idle.conf.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

/** Past the default 10-minute threshold by far: auto-away holds from unlock. */
export const IDLE_SECONDS = 100_000;

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/idle/**/*.e2e.ts"],
  capabilities: [
    {
      [APP_CAPABILITY]: { window: "login", display: "native", env: { REKINDLE_WDIO_IDLE_SECONDS: String(IDLE_SECONDS) } } satisfies AppCapability,
    } as WebdriverIO.Capabilities,
  ],
};
