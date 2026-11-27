// Contract PN1 (its addendum's requestToSpeak key): a stage's raise hand is
// never hidden. Cara owns the community and a stage channel, and denies
// everyone REQUEST_TO_SPEAK there (the bit her own catalogue names). Ari
// joins the stage as audience: "Request to speak" is shown, disabled, with
// the Host's reason as its title, in the theme's disabled text. At the
// default window size, the Audience header and Ari's own row show without a
// scroll: inside every clipping ancestor's box and the window. Joining a
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

    // The Audience header and our own row are in view, unscrolled: each
    // inside the box every clipping ancestor (and the window) leaves visible.
    await shown(browser, ".stage-panel-section .stage-panel-member-card", NET_WAIT_MS);
    const fit = (await browser.execute(() => {
      const clip = (el: Element): DOMRect => {
        let box = new DOMRect(0, 0, window.innerWidth, window.innerHeight);
        for (let a = el.parentElement; a; a = a.parentElement) {
          const s = getComputedStyle(a);
          if (s.overflowX === "visible" && s.overflowY === "visible") continue;
          const r = a.getBoundingClientRect();
          const left = Math.max(box.left, r.left);
          const top = Math.max(box.top, r.top);
          box = new DOMRect(left, top, Math.min(box.right, r.right) - left, Math.min(box.bottom, r.bottom) - top);
        }
        return box;
      };
      const inside = (el: Element | null | undefined): { inside: boolean; el: number[]; clip: number[] } => {
        if (!el) return { inside: false, el: [], clip: [] };
        const r = el.getBoundingClientRect();
        const c = clip(el);
        const round = (b: DOMRect) => [b.left, b.top, b.right, b.bottom].map(Math.round);
        return { inside: r.top >= c.top - 0.5 && r.bottom <= c.bottom + 0.5 && r.left >= c.left - 0.5 && r.right <= c.right + 0.5, el: round(r), clip: round(c) };
      };
      const sections = Array.from(document.querySelectorAll(".stage-panel-grid .stage-panel-section"));
      const audience = sections.find((s) => (s.querySelector(".stage-panel-section-title")?.textContent ?? "").includes("Audience"));
      return {
        header: inside(audience?.querySelector(".stage-panel-section-title")),
        row: inside(audience?.querySelector(".stage-panel-member-card")),
        scrolled: document.querySelector(".stage-panel")?.scrollTop ?? -1,
        window: [window.innerWidth, window.innerHeight],
      };
    })) as { header: { inside: boolean; el: number[]; clip: number[] }; row: { inside: boolean; el: number[]; clip: number[] }; scrolled: number; window: number[] };
    console.log(`[stage] window ${fit.window.join("x")}, panel scrolled ${fit.scrolled}; Audience header ${JSON.stringify(fit.header)}; our row ${JSON.stringify(fit.row)}`);
    await shot("request-to-speak-denied");
    expect({ shown: await request.isDisplayed(), disabled: state.disabled, reason: state.reason.length > 0, dimmed: state.dimmed })
      .toEqual({ shown: true, disabled: true, reason: true, dimmed: true });
    expect({ scrolled: fit.scrolled, header: fit.header.inside, row: fit.row.inside }).toEqual({ scrolled: 0, header: true, row: true });
  });
});
