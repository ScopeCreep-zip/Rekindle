// Plan E3.2 S10, GUI half: a role change reaches the community window live
// (EffectivePermissionsChanged), with no reopening. Cara (fleet) creates a
// community; Ari (the app) joins as a plain member and keeps its window
// open on general. The channel list's "+ Category" control is drawn only
// for MANAGE_CHANNELS: absent (shot); Cara grants Ari a role with it and the
// control appears in the open window (shot); Cara takes the role back and
// it goes again (shot).

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAssignRole, peerCreateCommunity, peerCreateRole, peerInvite, peerMemberCount, peerUnassignRole, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Perm Watch ${Date.now() % 100000}`;
/** rekindle_types::permissions::MANAGE_CHANNELS. */
const MANAGE_CHANNELS = 1 << 1;
const CONTROL = ".channel-create-category-btn";

/** Whether the open community window draws the MANAGE_CHANNELS control. */
async function controlShown(): Promise<boolean> {
  return (await browser.execute((sel: string) => {
    const el = document.querySelector<HTMLElement>(sel);
    return !!el && el.getBoundingClientRect().width > 0;
  }, CONTROL)) as boolean;
}

describe("a role granted or taken back changes the open community window live", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("permlive"), displayName: "Ari" };
  let peers: Peer[] = [];
  let anchor = "";
  let role = "";

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    anchor = await peerCreateCommunity(peers[0], COMMUNITY);
    await joinCommunity(browser, me, await peerInvite(peers[0], anchor, 1), COMMUNITY);
    await waitUntil(async () => (await peerMemberCount(peers[0], anchor)) >= 2, NET_WAIT_MS * 2, "Cara never counted Ari");
    await openCommunity(browser, COMMUNITY);
    await selectChannel(browser, "general");
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("a plain member has no channel management control", async () => {
    await browser.pause(2000);
    if (await controlShown()) throw new Error("Ari, a plain member, already has the Manage Channels control");
    await shot("before-grant");
  });

  it("granted Manage Channels, the control appears in the open window", async () => {
    role = await peerCreateRole(peers[0], anchor, "Builders", MANAGE_CHANNELS);
    await peerAssignRole(peers[0], anchor, me.displayName, role);
    await waitUntil(controlShown, NET_WAIT_MS * 2, "the Manage Channels control never appeared after the grant (no reopening)");
    await shot("after-grant");
  });

  it("the role taken back, the control goes again", async () => {
    await peerUnassignRole(peers[0], anchor, me.displayName, role);
    await waitUntil(async () => !(await controlShown()), NET_WAIT_MS * 2, "the Manage Channels control stayed after the role was taken back");
    await shot("after-revoke");
  });
});
