// The member profile popup stays inside the community window (lead,
// 2026-11-27; PF2's shots showed it running past the right edge, and the
// whole page scrolled sideways so the left column was clipped). Ari opens her
// own row in the member list, at the window's right edge: the popup's box is
// inside the viewport and nothing scrolls sideways (page, body, root), then
// the same once "Edit profile" makes the popup tall.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, press, shown } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

interface Fit {
  box: { left: number; top: number; right: number; bottom: number };
  viewport: { width: number; height: number };
  scroll: { page: number; body: number; root: number };
  /** What Kobalte measured and set: its positioner's inline style (the transform and the fitViewport variables) and the popup's resolved limits. */
  kobalte: { positioner: string; maxWidth: string; maxHeight: string; height: string };
}

async function fit(): Promise<Fit> {
  return (await browser.execute(() => {
    const el = document.querySelector(".profile-popup");
    const r = el?.getBoundingClientRect();
    const cs = el ? getComputedStyle(el) : null;
    return {
      box: { left: Math.round(r?.left ?? -1), top: Math.round(r?.top ?? -1), right: Math.round(r?.right ?? -1), bottom: Math.round(r?.bottom ?? -1) },
      viewport: { width: window.innerWidth, height: window.innerHeight },
      scroll: {
        page: document.scrollingElement?.scrollLeft ?? 0,
        body: document.body.scrollLeft,
        root: document.getElementById("root")?.scrollLeft ?? 0,
      },
      kobalte: {
        positioner: el?.parentElement?.getAttribute("style") ?? "(none)",
        maxWidth: cs?.maxWidth ?? "",
        maxHeight: cs?.maxHeight ?? "",
        height: cs?.height ?? "",
      },
    };
  })) as Fit;
}

function inside(f: Fit): { inside: boolean; noScroll: boolean } {
  const { box, viewport, scroll } = f;
  return {
    inside: box.left >= 0 && box.top >= 0 && box.right <= viewport.width && box.bottom <= viewport.height,
    noScroll: scroll.page === 0 && scroll.body === 0 && scroll.root === 0,
  };
}

describe("the member profile popup stays inside the community window", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("popfit"), displayName: "Ari" };

  it("opened from a row at the window's right edge, closed and while editing", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Fit Hall");
    await openCommunity(browser, "Fit Hall");
    await selectChannel(browser, "general");
    await press(browser, await shown(browser, ".member-item[data-pseudonym]", NET_WAIT_MS));
    await shown(browser, ".profile-popup", NET_WAIT_MS);
    const opened = await fit();
    console.log(`[popfit] opened: ${JSON.stringify(opened)}`);
    await shot("popup-opened");
    await press(browser, await byText(browser, ".profile-popup-action-btn", "Edit profile"));
    await shown(browser, ".profile-popup-edit-form", NET_WAIT_MS);
    const editing = await fit();
    console.log(`[popfit] editing: ${JSON.stringify(editing)}`);
    await shot("popup-editing");
    // Why Kobalte kept its first position (its transform did not move when
    // the editor grew): a second later (a late update), then after a window
    // resize event (floating-ui's autoUpdate listens for it, beside its
    // ResizeObserver on the positioner).
    await browser.pause(1000);
    console.log(`[popfit] editing +1 s: ${JSON.stringify(await fit())}`);
    await browser.execute(() => window.dispatchEvent(new Event("resize")));
    await browser.pause(500);
    console.log(`[popfit] after a resize event: ${JSON.stringify(await fit())}`);
    expect({ opened: inside(opened), editing: inside(editing) }).toEqual({
      opened: { inside: true, noScroll: true },
      editing: { inside: true, noScroll: true },
    });
  });
});
