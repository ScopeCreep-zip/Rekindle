// Leaving a community removes its row live, in every window holding the
// list (the store kept a deleted key: a setter given a copy without it
// merged it back). Cara (fleet) creates a community and Ari (the app) joins;
// with the community window open, Ari presses Leave. The buddy list's
// Communities list must no longer show it, without reopening anything.
// Before and after shots.

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, press, resolveIntent, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY } from "../../../walk/nav.ts";
import { type Peer, peerCreateCommunity, peerInvite, startFleet, stopFleet } from "../../fleet.ts";
import { communitiesTab, createIdentity, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Leave Hall ${Date.now() % 100000}`;

/** Whether the buddy list's Communities list shows the community. */
async function listed(): Promise<boolean> {
  return (await browser.execute(
    (n: string) => Array.from(document.querySelectorAll(".community-item")).some((r) => (r.textContent ?? "").includes(n)),
    COMMUNITY.slice(0, 24),
  )) as boolean;
}

describe("leaving a community removes its row live", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("leavelive"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari leaves from the community window and the buddy list drops the row", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    const anchor = await peerCreateCommunity(peers[0], COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(peers[0], anchor, 1), COMMUNITY);
    await communitiesTab(browser);
    if (!(await listed())) throw new Error(`${COMMUNITY} was never listed after joining`);
    await shot("before-leave");
    await openCommunity(browser, COMMUNITY);
    // The sidebar's Search shares Leave's class (its styling): the Leave button by its word.
    await press(browser, await shown(browser, "button.community-leave-btn*=Leave", NET_WAIT_MS));
    await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 1500 }).catch(() => undefined);
    await switchTo(browser, BUDDY);
    await communitiesTab(browser);
    await waitUntil(async () => !(await listed()), NET_WAIT_MS, `${COMMUNITY} stayed in the buddy list after leaving`);
    await shot("after-leave");
  });
});
