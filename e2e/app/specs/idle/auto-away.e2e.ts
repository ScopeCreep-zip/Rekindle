// AA1: auto-away fires and turning it off brings us back. The app launches
// with the idle override past the threshold (wdio.idle.conf.ts), so
// auto-away holds from unlock. Ari keeps auto-away on, creates a community
// (she is its only member, so its member list's one row is hers) and the
// spec checks, in order:
//   1. auto-away fired: her own member row reads Away (the precondition);
//   2. while away, the buddy list's own status agrees with the row;
//   3. turning auto-away off restores Online on the row;
//   4. then the buddy list and the row agree.
// Every check is recorded and the spec fails with each one that did not
// hold, so a control run says which. Each state a shot.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { DOT_STATUS, autoAwayOff, createCommunity, createIdentity, newUser, openCommunity, ownStatusValue, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Away Hall ${Date.now() % 100000}`;

describe("auto-away fires, and turning it off brings us back (AA1)", function () {
  this.timeout(900_000);
  const me: User = { ...newUser("autoaway"), displayName: "Ari" };
  let communityHandle = "";

  /** Ari's own member row's status in the community window, as a status value. */
  async function rowStatus(): Promise<string> {
    const here = await browser.getWindowHandle();
    await browser.switchToWindow(communityHandle);
    const label = (await browser.execute(
      () => document.querySelector('.member-item [role="img"]')?.getAttribute("aria-label") ?? "",
    )) as string;
    await browser.switchToWindow(here);
    return DOT_STATUS[label] ?? `"${label}"`;
  }

  it("Away while idle, Online once auto-away is off, the buddy list and the member row agreeing", async () => {
    const problems: string[] = [];
    await createIdentity(browser, me, { keepAutoAway: true });
    await createCommunity(browser, me, COMMUNITY);
    await openCommunity(browser, COMMUNITY);
    communityHandle = await browser.getWindowHandle();

    // 1. The precondition: auto-away fired.
    let row = "";
    await waitUntil(async () => (row = await rowStatus()) === "away", NET_WAIT_MS, "").catch(() => undefined);
    await shot("auto-away");
    if (row !== "away") throw new Error(`auto-away never fired: the member row reads ${row} (not provoked)`);

    // 2. While away, the buddy list agrees.
    await switchTo(browser, BUDDY);
    const ownAway = await ownStatusValue(browser);
    console.log(`[aa] while idle: member row ${row}, buddy list ${ownAway}`);
    await shot("buddy-while-away");
    if (ownAway !== row) problems.push(`while away, the buddy list reads ${ownAway} but the member row ${row}`);

    // 3. Turning auto-away off restores Online.
    await autoAwayOff(browser, me);
    await waitUntil(async () => (await rowStatus()) === "online", NET_WAIT_MS, "").catch(() => undefined);
    const rowAfter = await rowStatus();
    await browser.switchToWindow(communityHandle);
    await shot("after-auto-away-off");
    await switchTo(browser, BUDDY);
    if (rowAfter !== "online") problems.push(`after auto-away was turned off, the member row still reads ${rowAfter} (Online not restored)`);

    // 4. Then the buddy list and the row agree.
    const ownAfter = await ownStatusValue(browser);
    console.log(`[aa] after auto-away off: member row ${rowAfter}, buddy list ${ownAfter}`);
    await shot("buddy-after");
    if (ownAfter !== rowAfter) problems.push(`after auto-away off, the buddy list reads ${ownAfter} but the member row ${rowAfter}`);

    if (problems.length > 0) throw new Error(problems.join("; "));
  });
});
