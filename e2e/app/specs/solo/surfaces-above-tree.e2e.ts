// The layer contract (VN2, skin-tree.css): a pack's skin tree is its own
// stacking context, so its nodes (z-index 3 x the pack's Z, up to 297) order
// among themselves only and never paint over a Host-driven surface. The
// incoming-call banner was laid out but unseen beside Symbiosis's tree; this
// proves the other surfaces a skinned window raises are seen, by the owner's
// rule (hiding the part changes its pixels, painted.ts), in Symbiosis and in
// Alloy. One intent in a community window raises three of them: the channel
// topic's review (the intent dialog, every window's own), the undo offer it
// leaves (a toast), and the publication banner (in the pack's chat area
// slot) while the change waits for the network.

import { browser, expect } from "@wdio/globals";
import { gone, NET_WAIT_MS, resolveIntent, shown, typeInto } from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import {
  createCommunity,
  createIdentity,
  newUser,
  openCommunity,
  selectChannel,
  type User,
} from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { painted } from "../../painted.ts";
import { shot } from "../../shots.ts";

const b = browser;
const PLACE = "Layer Hall";
const PACKS = ["Symbiosis", "Alloy"];

/** `selector` shown, then whether it is seen (≥ half its box changes when hidden), logged. */
async function seen(pack: string, selector: string): Promise<boolean> {
  await shown(b, selector, NET_WAIT_MS);
  const paint = await painted(b, selector);
  console.log(`[layers] ${pack} ${selector}: ${JSON.stringify(paint)}`);
  return paint.total > 0 && paint.changed >= paint.total / 2;
}

describe("Host surfaces paint above a skin tree (layer contract)", function () {
  this.timeout(900_000);
  const ada: User = { ...newUser("layers"), displayName: "Ada" };

  it("the intent review, its undo toast and the publication banner are seen in Symbiosis and Alloy", async () => {
    await createIdentity(b, ada);
    await createCommunity(b, ada, PLACE);
    const unseen: string[] = [];
    for (const pack of PACKS) {
      await openSettings(b, "Appearance");
      const href = await applySkin(b, ada, pack);
      await openCommunity(b, PLACE);
      await skinShown(b, href);
      await selectChannel(b, "general");
      await gone(b, ".governance-publication", NET_WAIT_MS);
      await gone(b, ".toast-item", NET_WAIT_MS);

      // The topic's review, then the undo offer and the publication it leaves.
      await (await shown(b, ".channel-topic")).click();
      await typeInto(await shown(b, ".channel-topic-header-input"), `${pack} topic`);
      await b.keys("Enter");
      if (!(await seen(pack, ".intent-dialog"))) unseen.push(`${pack} intent review`);
      await shot(`review-${pack.toLowerCase()}`);
      await resolveIntent(b, { passphrase: ada.passphrase, appearMs: 3000 });
      if (!(await seen(pack, ".toast-item"))) unseen.push(`${pack} toast`);
      if (!(await seen(pack, ".governance-publication"))) unseen.push(`${pack} publication banner`);
      await shot(`publication-${pack.toLowerCase()}`);
    }
    expect(unseen).toEqual([]);
  });
});
