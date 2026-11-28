// The layer contract (VN2, skin-tree.css): a pack's skin tree is its own
// stacking context, so its nodes (z-index 3 x the pack's Z, up to 297) order
// among themselves only and never paint over a Host-driven surface. The
// incoming-call banner was laid out but unseen beside Symbiosis's tree; this
// proves the other surfaces a skinned window raises are seen, by the owner's
// rule (hiding the part changes its pixels, painted.ts), in Symbiosis and in
// Alloy. In a community window, a role made (a Routine intent, which runs
// without review and leaves a toast) then deleted (Consequential: the
// intent dialog's review, every window's own) raises all three, the
// deletion's op showing the publication banner (in the pack's chat area
// slot) while it waits for the network.

import { browser, expect } from "@wdio/globals";
import {
  byText,
  gone,
  NET_WAIT_MS,
  resolveIntent,
  shown,
  typeInto,
} from "../../../walk/helpers.ts";
import { openSettings } from "../../../walk/nav.ts";
import {
  communitySettings,
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

/** `selector` shown, then whether it is seen (hiding it changes its pixels: ≥ 5% of its box and 50 px), logged. */
async function seen(pack: string, selector: string): Promise<boolean> {
  await shown(b, selector, NET_WAIT_MS);
  const paint = await painted(b, selector);
  console.log(`[layers] ${pack} ${selector}: ${JSON.stringify(paint)}`);
  // A surface painted under the tree changes next to no pixels when hidden.
  // A text strip on the same ground as the area beneath it (the publication
  // banner) changes only its glyphs, about a tenth of its box, and is seen;
  // so seen is at least 5% of the box and 50 px, never half of it.
  return paint.total > 0 && paint.changed >= Math.max(50, paint.total * 0.05);
}

describe("Host surfaces paint above a skin tree (layer contract)", function () {
  this.timeout(900_000);
  const ada: User = { ...newUser("layers"), displayName: "Ada" };

  it("a toast, the intent review and the publication banner are seen in Symbiosis and Alloy", async () => {
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

      // A role made (Routine: no review; it leaves a toast), then deleted
      // (Consequential: the review), its op publishing in the chat area.
      const role = `${pack} role`;
      await communitySettings(b, "Roles");
      await (await byText(b, ".modal-container .form-btn-secondary", "Create Role")).click();
      await typeInto(await shown(b, 'input[placeholder="Role name..."]'), role);
      await (await byText(b, ".modal-container .form-btn-primary", "Create")).click();
      await resolveIntent(b, { passphrase: ada.passphrase, appearMs: 3000 });
      if (!(await seen(pack, ".toast-item"))) unseen.push(`${pack} toast`);
      const row = await byText(b, ".settings-role-row", role, NET_WAIT_MS);
      await (await row.$('button[title="Delete Role"]')).click();
      if (!(await seen(pack, ".intent-dialog"))) unseen.push(`${pack} intent review`);
      await shot(`review-${pack.toLowerCase()}`);
      await resolveIntent(b, { passphrase: ada.passphrase, appearMs: 3000 });
      await (await shown(b, ".modal-container .modal-close-btn")).click();
      await gone(b, ".modal-container");
      if (!(await seen(pack, ".governance-publication"))) unseen.push(`${pack} publication banner`);
      await shot(`publication-${pack.toLowerCase()}`);
    }
    expect(unseen).toEqual([]);
  });
});
