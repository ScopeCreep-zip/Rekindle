// Plan E7.14 / RV1 (owner decision 65): a paused inbound route is said in the
// buddy list's status area until the Host lifts it, and no control goes away
// for it (decision 49). The app runs with an empty route budget
// (wdio.budget.conf.ts), so the Host pauses the route at unlock with the
// cause "exhausted". The default skin (Symbiosis) lays the buddy list as its
// tree, whose status area is the pack's status field (StatusField.tsx).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, underTree } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";

/** The Host's cause "exhausted", worded (network.store.ts routePausedText). */
const EXHAUSTED = "Inbound route paused after repeated failures: possible attack";

const PAUSED = ".skin-status-paused[role=alert]";
const FIELD = ".skin-status-edit";

describe("a paused inbound route is said in the tree's status field, which stays usable", function () {
  this.timeout(600_000);
  const me: User = newUser("paused");

  before(async () => {
    await createIdentity(browser, me);
    await switchTo(browser, BUDDY);
    if (!(await underTree(browser))) throw new Error("the default skin no longer lays the buddy list as a tree");
  });

  it("the status field says the Host's cause, whole, as an alert", async () => {
    const line = await shown(browser, PAUSED, NET_WAIT_MS);
    expect((await line.getText()).trim()).toBe(EXHAUSTED);
    // The whole line is the field's tooltip too: the pack's box ellipsizes it.
    expect(await (await browser.$(FIELD)).getAttribute("title")).toBe(EXHAUSTED);
  });

  it("the away message can still be typed: the line steps aside while the field has focus, then returns", async () => {
    const field = await browser.$(FIELD);
    await field.click();
    await gone(browser, PAUSED, NET_WAIT_MS);
    const away = "back after the reboot";
    await field.setValue(away);
    await browser.keys(["Enter"]);
    await browser.execute(() => (document.activeElement as HTMLElement | null)?.blur());
    await shown(browser, PAUSED, NET_WAIT_MS);
    // Set, not dropped: the field shows the Host's stored message after blur.
    await waitUntil(async () => (await field.getValue()) === away, NET_WAIT_MS, "the away message typed while paused was not kept");
  });
});
