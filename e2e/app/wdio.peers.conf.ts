// Plan E7.14 S2: the real-app suite's specs with several users on this
// machine. Multiremote (webdriver.io/docs/multiremote): one worker holds a
// session per named user, and `RekindleAppLauncher` starts each user's own
// app (data root, port, token, Host and Veilid node). A spec reaches a user
// by name: `multiRemoteBrowser.getInstance("alice")`.

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

const user = { capabilities: { [APP_CAPABILITY]: { window: "login", display: "native" } satisfies AppCapability } as WebdriverIO.Capabilities };

export const config: WebdriverIO.MultiRemoteConfig = {
  ...base,
  specs: ["./specs/peers/**/*.e2e.ts"],
  capabilities: { alice: user, bob: structuredClone(user) },
};
