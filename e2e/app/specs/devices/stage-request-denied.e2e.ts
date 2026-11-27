// Contract PN1 (its addendum's requestToSpeak key): a stage's raise hand is
// never hidden. Cara owns the community and a stage channel, and denies
// everyone REQUEST_TO_SPEAK there (the bit her own catalogue names). Ari
// joins the stage as audience: "Request to speak" is shown, disabled, with
// the Host's reason as its title, in the theme's disabled text. Joining a
// stage opens the microphone: Linux only (no TCC prompt on the Mac).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown } from "../../../walk/helpers.ts";
import { type Peer, peerCreateCommunity, peerInvite, peerReviewed, startFleet, stopFleet } from "../../fleet.ts";
import { createIdentity, joinCommunity, newUser, openCommunity, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

const COMMUNITY = `Stage Hall ${Date.now() % 100000}`;
const STAGE = "town-hall";

interface Catalogue {
  groups: { permissions: { key: string; bit: string }[] }[];
}

describe("a stage's raise hand is shown, disabled with the Host's reason (PN1)", function () {
  this.timeout(1_800_000);
  const me: User = { ...newUser("stage"), displayName: "Ari" };
  let peers: Peer[] = [];

  after(async () => {
    await stopFleet(peers);
  });

  it("Request to speak is disabled with the reason where requestToSpeak is denied", async () => {
    await createIdentity(browser, me);
    peers = await startFleet(["Cara"]);
    const [cara] = peers;
    const anchor = await peerCreateCommunity(cara!, COMMUNITY);
    const catalogue = (await cara!.cli(["community", "permissions"])) as Catalogue;
    const bit = catalogue.groups.flatMap((g) => g.permissions).find((p) => p.key === "REQUEST_TO_SPEAK")?.bit;
    if (!bit) throw new Error("the catalogue names no REQUEST_TO_SPEAK");
    const made = (await peerReviewed(cara!, ["channel", "create", "-c", anchor, "-n", STAGE, "--kind", "stage"])) as { id?: string } | null;
    if (!made?.id) throw new Error("the Host did not return the stage's id");
    await peerReviewed(cara!, [
      "community", "channel", "overwrite", "set", "-c", anchor, "-C", made.id,
      "--target", "everyone", "--deny", `0x${BigInt(bit).toString(16)}`,
    ]);
    await joinCommunity(browser, me, await peerInvite(cara!, anchor), COMMUNITY);
    await openCommunity(browser, COMMUNITY);

    await (await byText(browser, ".channel-item", STAGE, NET_WAIT_MS)).click();
    await press(browser, await shown(browser, ".stage-panel-join-btn", NET_WAIT_MS));
    const request = await shown(browser, ".stage-panel-request-btn", NET_WAIT_MS);
    const state = (await browser.execute(() => {
      const btn = document.querySelector(".stage-panel-request-btn");
      const probe = document.createElement("span");
      probe.style.color = "var(--color-xfire-text-disabled)";
      document.body.appendChild(probe);
      const disabledColour = getComputedStyle(probe).color;
      probe.remove();
      return {
        disabled: (btn as HTMLButtonElement | null)?.disabled ?? false,
        reason: btn?.getAttribute("title") ?? "",
        dimmed: !!btn && getComputedStyle(btn).color === disabledColour,
      };
    })) as { disabled: boolean; reason: string; dimmed: boolean };
    console.log(`[stage] Request to speak: disabled=${state.disabled} reason=${state.reason}`);
    await shot("request-to-speak-denied");
    expect({ shown: await request.isDisplayed(), disabled: state.disabled, reason: state.reason.length > 0, dimmed: state.dimmed })
      .toEqual({ shown: true, disabled: true, reason: true, dimmed: true });
  });
});
