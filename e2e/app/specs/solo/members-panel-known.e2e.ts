// The long-open "MEMBERS — 0": a community window drew its member panel
// before its first member read answered, and showed "Members — 0" for a
// community the user is in (the Host served the creator all along). Now the
// panel shows no count until the Host's first answer. Watched from the moment
// the window is switched to: every header the panel shows is recorded, none
// may be "Members — 0", and it settles on "Members — 1" with Ari's row.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, communityWindowLabel, doubleClick, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { communitiesTab, createCommunity, createIdentity, newUser, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Known Hall ${Date.now() % 100000}`;

describe("the member panel never claims 0 members for a community we're in", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("known"), displayName: "Ari" };

  it("from the window's first moment the header is either countless or our real count", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, COMMUNITY);
    await inBuddy(browser);
    const label = await communityWindowLabel(browser, COMMUNITY);
    await communitiesTab(browser);
    await doubleClick(browser, await byText(browser, ".community-item", COMMUNITY.slice(0, 24), NET_WAIT_MS));
    await switchTo(browser, label, NET_WAIT_MS);
    // Every header the panel shows from here, in order.
    await browser.execute(() => {
      const seen: string[] = [];
      (window as unknown as { __headers: string[] }).__headers = seen;
      const read = (): string => document.querySelector(".member-list-header")?.textContent?.trim() ?? "";
      let last = read();
      if (last) seen.push(last);
      new MutationObserver(() => {
        const now = read();
        if (now && now !== last) seen.push(now);
        last = now;
      }).observe(document.body, { childList: true, subtree: true, characterData: true });
    });
    await shown(browser, ".member-list-header", NET_WAIT_MS);
    await waitUntil(
      async () => (await (await browser.$(".member-list-header")).getText()).trim().toLowerCase() === "members — 1",
      NET_WAIT_MS,
      "the member panel never showed Members — 1",
    );
    await shown(browser, '.member-item[data-pseudonym]', NET_WAIT_MS);
    const seen = (await browser.execute(() => (window as unknown as { __headers?: string[] }).__headers ?? [])) as string[];
    console.log(`[known] headers shown: ${JSON.stringify(seen)}`);
    await shot("members-known");
    const zero = seen.filter((h) => /—\s*0$/.test(h));
    const rows = (await browser.$$(".member-item[data-pseudonym]")).length;
    expect({ zero, rows }).toEqual({ zero: [], rows: 1 });
  });
});
