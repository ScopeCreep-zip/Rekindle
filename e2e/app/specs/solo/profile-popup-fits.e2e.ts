// The member profile popup stays inside the community window (lead,
// 2026-11-27; PF2's shots showed it running past the right edge, and the
// whole page scrolled sideways so the left column was clipped). Ari opens her
// own row in the member list, at the window's right edge: the popup's box is
// inside the viewport and nothing scrolls sideways (page, body, root). Kobalte
// places a popover only as it opens, so "Edit profile" opens the editor in its
// own dialog (lead's ruling): the dialog fits the window, its Save button is
// reachable inside it, focus moves in, and closing returns focus to our row.

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, byText, gone, press, shown } from "../../../walk/helpers.ts";
import { createCommunity, createIdentity, newUser, openCommunity, selectChannel, type User } from "../../journeys.ts";
import { shot } from "../../shots.ts";

interface Fit {
  box: { left: number; top: number; right: number; bottom: number };
  viewport: { width: number; height: number };
  scroll: { page: number; body: number; root: number };
}

async function fit(): Promise<Fit> {
  return (await browser.execute(() => {
    const r = document.querySelector(".profile-popup")?.getBoundingClientRect();
    return {
      box: { left: Math.round(r?.left ?? -1), top: Math.round(r?.top ?? -1), right: Math.round(r?.right ?? -1), bottom: Math.round(r?.bottom ?? -1) },
      viewport: { width: window.innerWidth, height: window.innerHeight },
      scroll: {
        page: document.scrollingElement?.scrollLeft ?? 0,
        body: document.body.scrollLeft,
        root: document.getElementById("root")?.scrollLeft ?? 0,
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

/** The editor dialog's box, whether its body scrolls, and its Save button's box once scrolled to. */
async function dialog(): Promise<{
  box: { left: number; top: number; right: number; bottom: number };
  viewport: { width: number; height: number };
  bodyScrolls: boolean;
  save: { top: number; bottom: number };
}> {
  return (await browser.execute(() => {
    const round = (r: DOMRect | undefined) => ({
      left: Math.round(r?.left ?? -1), top: Math.round(r?.top ?? -1), right: Math.round(r?.right ?? -1), bottom: Math.round(r?.bottom ?? -1),
    });
    const container = document.querySelector(".modal-container");
    const body = container?.querySelector(".modal-body");
    const save = container?.querySelector<HTMLElement>('.profile-editor-actions button[type="submit"]');
    save?.scrollIntoView({ block: "nearest" });
    const s = round(save?.getBoundingClientRect());
    return {
      box: round(container?.getBoundingClientRect()),
      viewport: { width: window.innerWidth, height: window.innerHeight },
      bodyScrolls: !!body && body.scrollHeight > body.clientHeight && getComputedStyle(body).overflowY === "auto",
      save: { top: s.top, bottom: s.bottom },
    };
  })) as never;
}

describe("the member profile popup stays inside the community window", function () {
  this.timeout(600_000);
  const me: User = { ...newUser("popfit"), displayName: "Ari" };

  it("opens inside the window, and Edit profile opens the editor as a dialog that fits", async () => {
    await createIdentity(browser, me);
    await createCommunity(browser, me, "Fit Hall");
    await openCommunity(browser, "Fit Hall");
    await selectChannel(browser, "general");
    const row = await shown(browser, ".member-item[data-pseudonym]", NET_WAIT_MS);
    const pseudonym = await row.getAttribute("data-pseudonym");
    await press(browser, row);
    await shown(browser, ".profile-popup", NET_WAIT_MS);
    const opened = await fit();
    console.log(`[popfit] opened: ${JSON.stringify(opened)}`);
    await shot("popup-opened");

    // Edit profile: the popout closes and the editor opens in its own dialog,
    // with focus inside it.
    await press(browser, await byText(browser, ".profile-popup-action-btn", "Edit profile"));
    await shown(browser, ".modal-container .profile-editor-form", NET_WAIT_MS);
    await gone(browser, ".profile-popup", NET_WAIT_MS);
    const focusInside = await browser.execute(() => !!document.activeElement?.closest(".modal-container"));
    const editor = await dialog();
    console.log(`[popfit] editor dialog: ${JSON.stringify(editor)} focus inside: ${focusInside}`);
    await shot("editor-dialog");

    // Closing it gives focus back to our row, the popout's trigger (WCAG 2.4.3).
    await browser.keys("Escape");
    await gone(browser, ".modal-container", NET_WAIT_MS);
    const focusBack = await browser.execute(() => document.activeElement?.getAttribute("data-pseudonym") ?? document.activeElement?.className ?? "");
    console.log(`[popfit] focus after closing: ${focusBack}`);

    const { box, viewport } = editor;
    expect({
      popup: inside(opened),
      dialogInside: box.left >= 0 && box.top >= 0 && box.right <= viewport.width && box.bottom <= viewport.height,
      saveReachable: editor.save.top >= box.top && editor.save.bottom <= box.bottom,
      focusInside,
      focusBack,
    }).toEqual({
      popup: { inside: true, noScroll: true },
      dialogInside: true,
      saveReachable: true,
      focusInside: true,
      focusBack: pseudonym,
    });
    console.log(`[popfit] the editor's body ${editor.bodyScrolls ? "scrolls" : "fits without scrolling"} at ${viewport.height} px`);
  });
});
