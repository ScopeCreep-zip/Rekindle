// KP: the real-app spec of a damaged channel post. The app is a debug
// WebDriver build launched with REKINDLE_WDIO_DAMAGE_NEXT_POST=1 (the
// Host's webdriver-only switch: the next channel post it receives has one
// ciphertext byte flipped before it is opened, so the real AEAD fails and
// the real damaged path runs: store, publish, history). Its own config,
// since every run of this app damages its first received post.
//
//   REKINDLE_APP_NO_BUILD=1 pnpm exec wdio run e2e/app/wdio.damage.conf.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

export const config: WebdriverIO.Config = {
  ...base,
  specs: ["./specs/damage/**/*.e2e.ts"],
  capabilities: [
    {
      [APP_CAPABILITY]: { window: "login", display: "native", env: { REKINDLE_WDIO_DAMAGE_NEXT_POST: "1" } } satisfies AppCapability,
    } as WebdriverIO.Capabilities,
  ],
};
