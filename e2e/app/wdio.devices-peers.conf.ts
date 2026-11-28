// Plan E7.14: the device specs with several users (calls between real
// apps). As wdio.devices.conf.ts: run on Linux, or on macOS only from the
// owner's own terminal, because macOS TCC refuses the microphone to an app a
// Claude session launched; the suite never raises a permission prompt. One
// app per named user, as wdio.peers.conf.ts.

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

const user = { capabilities: { [APP_CAPABILITY]: { window: "login", display: "x11" } satisfies AppCapability } as WebdriverIO.Capabilities };

export const config: WebdriverIO.MultiRemoteConfig = {
  ...base,
  specs: ["./specs/devices/call-end.e2e.ts", "./specs/devices/voice-roster-peers.e2e.ts"],
  capabilities: { alice: user, bob: structuredClone(user), carol: structuredClone(user) },
};
