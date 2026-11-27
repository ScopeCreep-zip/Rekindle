// Contract PN1 (its addendum's channel keys): what the user may do in a
// channel is the Host's, never a shell bit check. Cara owns the community and
// denies everyone two permissions, each by the bit her own catalogue names
// (`rekindle community permissions`): SEND_MESSAGES in "Quiet" and
// MENTION_EVERYONE in "No pings". Ari joins:
//   - in Quiet the composer is replaced by the Host's sendMessages reason;
//   - in No pings @everyone stays listed in the mention picker, disabled with
//     the Host's mentionEveryone reason, and picking it inserts nothing.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, shown } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerReviewed, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Denial Hall ${Date.now() % 100000}`;
const QUIET = "quiet";
const NO_PINGS = "no-pings";

interface Catalogue {
  groups: { permissions: { key: string; bit: string }[] }[];
}

/** The deny mask for `key`, from the owner's own catalogue. */
function mask(catalogue: Catalogue, key: string): string {
  const entry = catalogue.groups.flatMap((g) => g.permissions).find((p) => p.key === key);
  if (!entry) throw new Error(`the catalogue names no ${key}`);
  return `0x${BigInt(entry.bit).toString(16)}`;
}

async function createChannel(owner: Peer, anchor: string, name: string): Promise<string> {
  const result = await peerReviewed(owner, ["channel", "create", "-c", anchor, "-n", name, "--kind", "text"]);
  const id = (result as { id?: string } | null)?.id;
  if (!id || !/^[0-9a-f]{32}$/i.test(id)) throw new Error(`The Host did not return an id for ${name}`);
  return id;
}

describe("a channel's denials are the Host's (PN1)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("denials"), displayName: "Ari" };
  let peers: Peer[] = [];

  before(async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    const [cara] = peers;
    const anchor = await peerCreateCommunity(cara!, COMMUNITY);
    const catalogue = (await cara!.cli(["community", "permissions"])) as Catalogue;
    const quiet = await createChannel(cara!, anchor, QUIET);
    const noPings = await createChannel(cara!, anchor, NO_PINGS);
    for (const [channel, key] of [[quiet, "SEND_MESSAGES"], [noPings, "MENTION_EVERYONE"]] as const) {
      await peerReviewed(cara!, [
        "community", "channel", "overwrite", "set", "-c", anchor, "-C", channel,
        "--target", "everyone", "--deny", mask(catalogue, key),
      ]);
    }
    // Join after the overwrites, so Ari's first view of each channel carries them.
    await joinCommunity(browser, me, await peerInvite(cara!, anchor), COMMUNITY);
    await openCommunity(browser, COMMUNITY);
  });

  after(async () => {
    await stopFleet(peers);
  });

  it("says the Host's reason where we may not post", async () => {
    await selectChannel(browser, QUIET);
    const label = await shown(browser, ".message-input-wrapper .typing-label", NET_WAIT_MS);
    const said = (await label.getText()).trim();
    console.log(`[denials] composer in ${QUIET}: ${said}`);
    await shot("composer-denied");
    expect({
      reason: said.length > 0 && said !== "You cannot send messages here",
      noComposer: (await browser.$$(".message-input-wrapper textarea.message-input-field")).length === 0,
    }).toEqual({ reason: true, noComposer: true });
  });

  it("keeps @everyone listed with the Host's reason where we may not ping, and never inserts it", async () => {
    await selectChannel(browser, NO_PINGS);
    const box = await shown(browser, ".message-input-wrapper textarea.message-input-field", NET_WAIT_MS);
    await box.click();
    await browser.keys("@ev".split(""));
    const row = await byText(browser, ".mention-popover-row", "@everyone", NET_WAIT_MS);
    const reason = ((await (await row.$(".mention-popover-reason")).getText()) ?? "").trim();
    const disabled = await row.getAttribute("aria-disabled");
    console.log(`[denials] @everyone in ${NO_PINGS}: aria-disabled=${disabled} reason=${reason}`);
    await shot("everyone-denied");
    // Enter on the disabled row: nothing is inserted (given a moment to land if it were).
    await browser.keys("Enter");
    await browser.pause(500);
    const value = await (await browser.$(".message-input-wrapper textarea.message-input-field")).getValue();
    expect({ disabled, reason: reason.length > 0, inserted: value.includes("@everyone ") }).toEqual({ disabled: "true", reason: true, inserted: false });
  });
});
