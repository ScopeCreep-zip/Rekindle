// Plan E7.14 S5 (moved from e2e/skin-appearance.spec.ts and the walk's
// ST9/ST10/ST13): choosing an imported skin puts it on trial (E7.7.2). The
// Host shows it in every window and asks "Keep this skin?"; unanswered, it
// reverts to the stored skin when the trial runs out (host/skin/trial.rs:
// SKIN_TRIAL_MS from `reverts_at_ms`); kept, it stays. The trial's own 15 s
// runs for real: the Host's revert waits on tokio's clock, which the netsim
// controls but a real app cannot, so the spec waits the app's deadline plus
// a small margin, never a raised timeout.
//
// The imported package is a built-in pack's own file: an import is stored
// as `pkg:<BLAKE3 of the canonical package>` (host/skin/ids.rs), a library
// row apart from `builtin:<id>`, told apart here by the Remove it offers.

import { readFileSync } from "node:fs";
import path from "node:path";
import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, present, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { SETTINGS, openSettings, pickNextFile } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";

const PACKAGE = path.join(import.meta.dirname, "..", "..", "..", "..", "crates", "rekindle-skin", "builtin", "onyx.rkskin");
/** The trial's length (rekindle_types SKIN_TRIAL_MS); the deadline itself is read from the prompt. */
const TRIAL_MS = 15_000;
/** Slack past the deadline for the revert to reach the window. */
const MARGIN_MS = 5_000;

const SHEET = 'var l = document.getElementById("rekindle-skin"); return l && l.sheet ? l.getAttribute("href") : "";';

/** The library row (the one offering Remove) of the open Appearance tab. */
async function libraryRow(): Promise<Awaited<ReturnType<typeof browser.$>>> {
  for (const row of await browser.$$(".skin-row")) {
    for (const action of await row.$$(".skin-action")) if ((await action.getText()).trim() === "Remove") return row;
  }
  throw new Error("no imported skin is listed");
}

describe("a chosen imported skin is on trial until kept, and reverts when the trial runs out", function () {
  this.timeout(600_000);
  const me: User = newUser("trial");
  let stored = "";

  before(async () => {
    await createIdentity(browser, me);
    await openSettings(browser, "Appearance");
    await shown(browser, ".skin-row-shown .skin-name", NET_WAIT_MS);
    stored = (await browser.execute(SHEET)) as string;
    expect(stored.startsWith("skin://")).toBe(true);
    await pickNextFile(browser, "onyx.rkskin", "application/octet-stream", readFileSync(PACKAGE).toString("base64"));
    await (await byText(browser, "button.form-btn-secondary", "Import skin")).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 3000 });
    await waitUntil(
      async () => {
        const error = await browser.$(".skin-error");
        if ((await error.isExisting()) && (await error.isDisplayed())) throw new Error(`the import was refused: ${await error.getText()}`);
        return libraryRow().then(
          () => true,
          () => false,
        );
      },
      NET_WAIT_MS,
      "the imported skin was never listed",
    );
  });

  it("unanswered, the trial shows the skin, then reverts to the stored one when it runs out", async () => {
    await switchTo(browser, SETTINGS);
    const chosenAt = Date.now();
    await (await (await libraryRow()).$("input[type=radio]")).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await shown(browser, ".skin-trial", NET_WAIT_MS);
    let trialSheet = "";
    await waitUntil(
      async () => {
        trialSheet = (await browser.execute(SHEET)) as string;
        return trialSheet.startsWith("skin://") && trialSheet !== stored;
      },
      NET_WAIT_MS,
      "the trial's sheet was never applied",
    );
    // Not answered: the Host reverts at its deadline.
    await waitUntil(async () => (await browser.execute(SHEET)) === stored, TRIAL_MS + MARGIN_MS, "the skin did not revert when the trial ran out");
    expect(Date.now() - chosenAt).toBeGreaterThanOrEqual(TRIAL_MS - 1_000);
    expect(await present(browser, ".skin-trial")).toBe(false);
  });

  it("kept, the skin stays past the trial's end", async () => {
    await switchTo(browser, SETTINGS);
    await (await (await libraryRow()).$("input[type=radio]")).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await shown(browser, ".skin-trial", NET_WAIT_MS);
    await (await byText(browser, ".skin-trial button", "Keep this skin")).click();
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 });
    await waitUntil(async () => !(await present(browser, ".skin-trial")), NET_WAIT_MS, "the trial prompt stayed after Keep");
    const kept = (await browser.execute(SHEET)) as string;
    expect(kept).not.toBe(stored);
    await new Promise((r) => setTimeout(r, TRIAL_MS + MARGIN_MS));
    expect(await browser.execute(SHEET)).toBe(kept);
  });
});
