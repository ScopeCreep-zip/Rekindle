// Plan E3.2 S9: the Host emits BansChanged whenever a community's ban list
// changes, and Settings → Bans, while open, shows the change without being
// reopened. Cara (fleet) creates a community; Ari (the app) and Bea (fleet)
// join; Cara gives Ari a role that may ban (BAN_MEMBERS), so Ari's settings
// show the Bans tab. Ari opens it (no bans); Cara bans Bea by CLI; Ari's
// still-open list must show Bea. Each state a shot.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAssignRole, peerBan, peerCreateCommunity, peerCreateRole, peerInvite, peerJoinCommunity, peerMemberCount, startFleet, stopFleet } from "../../fleet.ts";
import { communitySettings, createIdentity, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Ban Watch ${Date.now() % 100000}`;
/** rekindle_types::permissions::BAN_MEMBERS. */
const BAN_MEMBERS = 1 << 6;

/** The names Ari's open Bans tab lists. */
async function bannedShown(): Promise<string[]> {
  return (await browser.execute(() =>
    Array.from(document.querySelectorAll(".modal-container .settings-list-item .settings-list-name")).map((n) => (n.textContent ?? "").trim()),
  )) as string[];
}

describe("Settings → Bans shows a ban made while it is open (BansChanged)", function () {
  this.timeout(2_400_000);
  const me: User = { ...newUser("bans"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara", "Bea"]);
    const [cara, bea] = peers;
    anchor = await peerCreateCommunity(cara, COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(cara, anchor, 1), COMMUNITY);
    await peerJoinCommunity(bea, await peerInvite(cara, anchor, 1));
    await waitUntil(async () => (await peerMemberCount(cara, anchor)) >= 3, NET_WAIT_MS * 2, "Cara never counted three members");
    const mods = await peerCreateRole(cara, anchor, "Mods", BAN_MEMBERS);
    await peerAssignRole(cara, anchor, me.displayName, mods);
    await openCommunity(browser, COMMUNITY);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("Ari's open ban list gains Bea when Cara bans her", async () => {
    // The role reaches Ari's window as a permission change: wait for the tab.
    await waitUntil(
      async () => {
        await communitySettings(browser, "Overview").catch(() => undefined);
        return (await byText(browser, ".modal-container .form-tab", "Bans", 1000).catch(() => null)) !== null;
      },
      NET_WAIT_MS,
      "Ari's settings never showed Bans (the Mods role did not arrive)",
    );
    await communitySettings(browser, "Bans");
    expect(await bannedShown()).not.toContain("Bea");
    await shot("bans-before");
    await peerBan(peers[0], anchor, "Bea");
    let shown: string[] = [];
    await waitUntil(
      async () => (shown = await bannedShown()).some((n) => n.includes("Bea")),
      NET_WAIT_MS * 2,
      "",
    ).catch(() => {
      throw new Error(`Ari's open ban list never showed Bea (shows ${JSON.stringify(shown)})`);
    });
    await shot("bans-after-live");
  });
});
