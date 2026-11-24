// Plan E3.2 S10, GUI half: a role change reaches the community window live
// (EffectivePermissionsChanged), with no reopening. Cara (fleet) creates a
// community; Ari (the app) joins as a plain member and keeps its window
// open on general. The channel list's "+ Category" is always drawn (never
// hide): disabled with the Host's reason (shot); Cara grants Ari a role with
// MANAGE_CHANNELS and it enables in the open window (shot); Cara takes the
// role back and it is disabled with a reason again (shot).

import { browser } from "@wdio/globals";
import { NET_WAIT_MS, waitUntil } from "../../../walk/helpers.ts";
import { type Peer, peerAssignRole, peerCreateCommunity, peerCreateRole, peerInvite, peerMemberCount, peerUnassignRole, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Perm Watch ${Date.now() % 100000}`;
/** rekindle_types::permissions::MANAGE_CHANNELS. */
const MANAGE_CHANNELS = 1 << 1;
const CONTROL = ".channel-create-category-btn";

interface ControlState {
  drawn: boolean;
  disabled: boolean;
  reason: string;
}

/** The MANAGE_CHANNELS control as drawn: present, disabled, and its reason (title / aria-description). */
async function control(): Promise<ControlState> {
  return (await browser.execute((sel: string) => {
    const el = document.querySelector<HTMLButtonElement>(sel);
    if (!el || el.getBoundingClientRect().width === 0) return { drawn: false, disabled: false, reason: "" };
    const disabled = el.disabled || el.getAttribute("aria-disabled") === "true";
    return { drawn: true, disabled, reason: disabled ? (el.getAttribute("aria-description") ?? el.title ?? "") : "" };
  }, CONTROL)) as ControlState;
}

/** Wait until the control is drawn and `enabled` or disabled with a reason. */
async function controlIs(enabled: boolean, what: string): Promise<ControlState> {
  let last: ControlState = { drawn: false, disabled: false, reason: "" };
  await waitUntil(
    async () => {
      last = await control();
      return last.drawn && (enabled ? !last.disabled : last.disabled && last.reason.trim().length > 0);
    },
    NET_WAIT_MS * 2,
    "",
  ).catch(() => {
    throw new Error(`${what}: the control is ${JSON.stringify(last)}`);
  });
  console.log(`[perms] ${what}: ${JSON.stringify(last)}`);
  return last;
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

  it("a plain member sees the channel management control disabled, with the Host's reason", async () => {
    await controlIs(false, "before the grant");
    await shot("before-grant");
  });

  it("granted Manage Channels, the control enables in the open window", async () => {
    role = await peerCreateRole(peers[0], anchor, "Builders", MANAGE_CHANNELS);
    await peerAssignRole(peers[0], anchor, me.displayName, role);
    await controlIs(true, "after the grant (no reopening)");
    await shot("after-grant");
  });

  it("the role taken back, the control is disabled with a reason again", async () => {
    await peerUnassignRole(peers[0], anchor, me.displayName, role);
    await controlIs(false, "after the role was taken back");
    await shot("after-revoke");
  });
});
