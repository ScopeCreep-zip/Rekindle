// Plan E7.14 (2026-11-20): the real-app specs of the windowing itself, on
// the session's own display (`display: "native"`): on a Wayland session the
// app runs as a Wayland client, so what the compositor grants (the size a
// window opens at, maximize) is what is checked. The size sweeps run under
// wdio.conf.ts on X11 instead (launcher.ts AppCapability).
//
//   REKINDLE_APP_NO_BUILD=1 pnpm exec wdio run e2e/app/wdio.native.conf.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/native/**/*.e2e.ts"],
  capabilities: [{ [APP_CAPABILITY]: { window: "login", display: "native" } satisfies AppCapability } as WebdriverIO.Capabilities],
};
