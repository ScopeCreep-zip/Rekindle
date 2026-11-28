// Plan E7.14 / RV1: the real-app specs of a spent route budget. The app is
// a debug build started with REKINDLE_ROUTE_BUDGET_CAPACITY=0 (debug builds
// only), so the Host's new-route budget starts empty: its first allocation
// is refused and RoutePaused { cause: exhausted } fires at unlock (owner
// decision 65). Its own config, since every app it starts is paused.
//
//   REKINDLE_APP_NO_BUILD=1 pnpm exec wdio run e2e/app/wdio.budget.conf.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/budget/**/*.e2e.ts"],
  capabilities: [
    {
      [APP_CAPABILITY]: { window: "login", display: "x11", env: { REKINDLE_ROUTE_BUDGET_CAPACITY: "0" } } satisfies AppCapability,
    } as WebdriverIO.Capabilities,
  ],
};
