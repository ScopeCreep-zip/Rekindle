// Round shot set: the default skin's buddy list and Community window. Ari
// creates a community; the buddy list (Communities listing it) and its
// Community window are shots.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { communitiesTab, createCommunity, createIdentity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot, shotAllWindows } from "../../shots.ts";

const COMMUNITY = `Round Hall ${Date.now() % 100000}`;

describe("the default skin's buddy list and community window", function () {
  this.timeout(900_000);
  const me: User = { ...newUser("defwin"), displayName: "Ari" };

  it("shows both windows", async () => {
    await createIdentity(browser, me);
    await shot("buddy-list-friends");
    await createCommunity(browser, me, COMMUNITY);
    await switchTo(browser, BUDDY);
    await communitiesTab(browser);
    const listed = (await browser.execute(
      (n: string) => Array.from(document.querySelectorAll(".community-item")).some((r) => (r.textContent ?? "").includes(n)),
      COMMUNITY,
    )) as boolean;
    await shot("buddy-list-communities");
    expect(listed).toBe(true);
    await openCommunity(browser, COMMUNITY);
    await browser.pause(3000);
    await shot("community-window");
    await shotAllWindows();
  });
});
