// Probe (lead, 2026-11-26): Settings sometimes opens blank ("no .form-tab
// containing 'Application'" in setup), on locked and unlocked sessions
// alike. This opens Settings from the buddy list's menu ten times and
// records, for each open, what its page holds: whether its document loaded,
// how much it rendered, whether the tab rail exists, the machine's state,
// and a shot. A blank open is counted, never thrown, so one run says how
// often and in what state.

import { browser, expect } from "@wdio/globals";
import { WAIT_MS, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, closeSettings, menu } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { machineState } from "../../machine.ts";
import { shot } from "../../shots.ts";

interface Page {
  ready: string;
  bodyChars: number;
  tabs: number;
  hasTauri: boolean;
  scripts: number;
  title: string;
}

/** What the current window's page holds now. */
async function page(): Promise<Page> {
  return (await browser.execute(() => ({
    ready: document.readyState,
    bodyChars: document.body?.innerHTML.length ?? -1,
    tabs: document.querySelectorAll(".form-tab").length,
    hasTauri: "__TAURI_INTERNALS__" in window,
    scripts: document.scripts.length,
    title: document.title,
  }))) as Page;
}

describe("Settings opens with its page (probe)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("setprobe"), displayName: "Ari" };

  before(async () => {
    // Auto-away stays on: turning it off opens Settings, the thing probed.
    await createIdentity(browser, me, { keepAutoAway: true });
  });

  it("Settings opened ten times from the buddy list's menu: each page's state", async () => {
    let blank = 0;
    for (let i = 1; i <= 10; i++) {
      await switchTo(browser, BUDDY);
      await menu(browser, "Rekindle", "Profile");
      await switchTo(browser, SETTINGS);
      let last: Page | null = null;
      const painted = await waitUntil(
        async () => {
          last = await page().catch(() => null);
          return last !== null && last.tabs > 0;
        },
        WAIT_MS,
        "",
      ).then(
        () => true,
        () => false,
      );
      if (!painted) blank += 1;
      console.log(`[settings-probe] open ${i}: ${painted ? "painted" : "BLANK"} ${JSON.stringify(last)} machine ${machineState()}`);
      await shot(`settings-open-${i}-${painted ? "painted" : "blank"}`);
      await closeSettings(browser);
    }
    console.log(`[settings-probe] blank opens: ${blank} of 10`);
    expect(blank).toBe(0);
  });
});
