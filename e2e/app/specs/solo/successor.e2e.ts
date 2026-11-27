// Plan E7.14 for E3.2 S7s.4 (ADR 0011 "Successor community", lead decisions
// (33), (34)): a community's creator re-creates it under a new anchor with
// an invite sealed to every current member, and each member is offered the
// successor. The app's user is the member in one community and the creator
// in the other; the other side is a real peer (fleet.ts: rekindled + CLI),
// so the offer comes over Veilid and the Host, never an injected event.
// (Rewritten 2026-11-26 from a draft preserved in wt/succui, c96917b52, onto
// today's fleet helpers: peerCreateCommunity, peerInvite, peerReviewed.)
//
// 1. A peer's community the app joined is re-created by the peer: the
//    buddy list shows "<community> moved to a new community" with Follow;
//    Follow goes through the Host's review, the row leaves, and the app
//    holds the successor as well as the old community. Before that, the
//    member's own Re-create is disabled and says why.
// 2. The app's own community, which the peer joined, is re-created from
//    Community Settings → Overview through the Host's review: the peer is
//    offered it (`community successors`), and the section shows it.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, resolveIntent, shown, sleep, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerJoinCommunity, peerReviewed, startFleet, stopFleet } from "../../fleet.ts";
import { communitySettings, createCommunity, createIdentity, createInvite, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { inBuddy } from "../../../walk/nav.ts";
import { showSkinView } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";

const PEERS_TOWN = "Moving Town";
const OUR_TOWN = "Rebuilt Town";

/** The communities `peer` holds named `name`. */
async function peerHolds(peer: Peer, name: string): Promise<number> {
  const list = (await peer.cli(["community", "list"])) as { name?: string }[] | null;
  return (list ?? []).filter((c) => c.name === name).length;
}

/** The app's buddy list Communities rows named `name` (the view opened by
 *  the pack's own control under a skin's tree, plan SK2-N4). */
async function appHolds(name: string): Promise<number> {
  await inBuddy(browser);
  await showSkinView(browser, "communities");
  const names = (await browser.execute(() =>
    Array.from(document.querySelectorAll(".community-item")).map((e) => (e.textContent ?? "").trim()),
  )) as string[];
  return names.filter((n) => n.includes(name)).length;
}

describe("a re-created community is offered to its members", function () {
  this.timeout(1_200_000);
  const me: User = newUser("successor");
  let peers: Peer[] = [];
  let founder: Peer;

  before(async () => {
    peers = await startFleet(["Founder"]);
    founder = peers[0]!;
    await createIdentity(browser, me);
  });
  after(async () => {
    await stopFleet(peers);
  });

  it("a member follows the peer's re-created community from the buddy list", async () => {
    const anchor = await peerCreateCommunity(founder, PEERS_TOWN);
    await joinCommunity(browser, me, await peerInvite(founder, anchor), PEERS_TOWN);
    // A member holds no creator seat (the Host's weAreCreator): Re-create is
    // disabled and says why, never an enabled button that only errors.
    await openCommunity(browser, PEERS_TOWN);
    await communitySettings(browser, "Overview");
    const recreate = await byText(browser, ".modal-container button", "Re-create community");
    expect(await recreate.isEnabled()).toBe(false);
    await byText(browser, ".modal-container .settings-hint", "Only a creator");
    await shot("member-recreate-disabled");
    await browser.closeWindow();

    await peerReviewed(founder, ["community", "publish-successor", "-c", anchor]);

    await inBuddy(browser);
    await showSkinView(browser, "friends");
    const row = await byText(browser, ".pending-item", "moved to a new community", NET_WAIT_MS);
    expect(await row.getText()).toContain(PEERS_TOWN);
    await shot("follow-offered");
    await (await row.$("button=Follow")).click();
    expect(await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 5000 })).toBe("committed");
    await waitUntil(
      async () => !(await (await browser.$(".pending-item*=moved to a new community")).isExisting()),
      NET_WAIT_MS,
      "the Follow row stayed after following",
    );
    await waitUntil(async () => (await appHolds(PEERS_TOWN)) >= 2, NET_WAIT_MS, "the app never held the successor beside the old community");
    await shot("successor-held");
  });

  it("the creator re-creates its community from Settings and the member is offered it", async () => {
    await createCommunity(browser, me, OUR_TOWN);
    await openCommunity(browser, OUR_TOWN);
    const link = await createInvite(browser, me, 1);
    await peerJoinCommunity(founder, link);
    await waitUntil(async () => (await peerHolds(founder, OUR_TOWN)) === 1, NET_WAIT_MS, "the peer never held the app's community");
    // The peer counts as a current member once its join reached our merge:
    // the successor continues exactly the members it holds (decision (34)).
    await sleep(5000);

    await communitySettings(browser, "Overview");
    await (await byText(browser, ".modal-container button", "Re-create community")).click();
    expect(await resolveIntent(browser, { passphrase: me.passphrase, appearMs: 5000 })).toBe("committed");
    await byText(browser, ".modal-container .settings-value", "Re-created:", NET_WAIT_MS);
    await shot("recreated-in-settings");

    let offered = "";
    await waitUntil(
      async () => {
        // The snapshot is `{ offers, link_requests }` (successor.rs
        // successor_snapshot), not a list.
        const snap = (await founder.cli(["community", "successors", "-c", OUR_TOWN])) as { offers?: unknown[] } | null;
        offered = JSON.stringify(snap).slice(0, 300);
        return (snap?.offers ?? []).length === 1;
      },
      NET_WAIT_MS,
      "the peer was never offered the re-created community",
    ).catch((e: unknown) => {
      throw new Error(`${(e as Error).message}; its last snapshot: ${offered}`);
    });
  });
});
