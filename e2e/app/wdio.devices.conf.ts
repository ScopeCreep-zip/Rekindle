// Plan E7.14: the real-app specs that need a camera or microphone (calls).
// Run apart from `pnpm test:app` as `pnpm test:app:devices`: on Linux, and
// on macOS only from the owner's own terminal, because macOS TCC grants
// device access to the launching app and refuses an app a Claude session
// launched. The suite never raises a permission prompt itself, and a
// refused device fails the spec loudly, never skips it.

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/devices/**/*.e2e.ts"],
  // Several users: wdio.devices-peers.conf.ts runs it.
  exclude: ["./specs/devices/call-end.e2e.ts"],
  capabilities: [{ [APP_CAPABILITY]: { window: "login", display: "x11" } satisfies AppCapability } as WebdriverIO.Capabilities],
};
