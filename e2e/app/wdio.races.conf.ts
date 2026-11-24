// Plan E6.11 / peer finding F5: the real-app specs of two answer races, each
// provoked every run rather than left to the network. Two users
// (multiremote, as wdio.peers.conf.ts), each a debug WebDriver build with
// REKINDLE_WDIO_ANSWER_DELAY (src-tauri/src/webdriver.rs): Alice's
// send_message answers 20 s after it commits, so the outbox's sent and the
// friend's receipt land first; Bob's get_friends answers 20 s after its
// read, so a friend's live status lands first. Its own config, since every
// answer of those commands is held.
//
//   REKINDLE_APP_NO_BUILD=1 pnpm exec wdio run e2e/app/wdio.races.conf.ts

import { base } from "./base.conf.ts";
import { APP_CAPABILITY, type AppCapability } from "./launcher.ts";

/** How long each held answer waits: past a live state on a loaded machine, under the hook's 30 s cap. */
export const HELD_MS = 20_000;

const user = (env: Record<string, string>) =>
  ({ capabilities: { [APP_CAPABILITY]: { window: "login", display: "native", env } satisfies AppCapability } as WebdriverIO.Capabilities });

export const config: WebdriverIO.MultiRemoteConfig = {
  ...base,
  specs: ["./specs/races/**/*.e2e.ts"],
  capabilities: {
    alice: user({ REKINDLE_WDIO_ANSWER_DELAY: `send_message=${HELD_MS}` }),
    bob: user({ REKINDLE_WDIO_ANSWER_DELAY: `get_friends=${HELD_MS}` }),
  },
};
