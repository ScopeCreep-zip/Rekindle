// Plan E7.14 / RV1 (owner decision 65): a paused inbound route is said in the
// buddy list's status area until the Host lifts it, and no control goes away
// for it (decision 49). The app runs with an empty route budget
// (wdio.budget.conf.ts), so the Host pauses the route at unlock with the
// cause "exhausted". The default skin (Symbiosis) lays the buddy list as its
// tree, whose status area is the pack's status field (StatusField.tsx).

import { browser, expect } from "@wdio/globals";
import { NET_WAIT_MS, gone, shown, switchTo, waitUntil } from "../../../walk/helpers.ts";
import { BUDDY, SETTINGS, closeSettings, openSettings, underTree } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { decodePng } from "../../png.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

/** The Host's cause "exhausted", worded (network.store.ts routePausedText). */
const EXHAUSTED = "Inbound route paused after repeated failures: possible attack";

const PAUSED = ".skin-status-paused[role=alert]";
const FIELD = ".skin-status-edit";

/**
 * How many pixels inside the status field's box change when `selector`
 * (the paused line or the input) is hidden: what of it is seen there.
 */
async function seenInField(selector: string): Promise<number> {
  const box = (await browser.execute(() => {
    const r = document.querySelector(".skin-status")?.getBoundingClientRect();
    return r ? [r.x, r.y, r.width, r.height, innerWidth] : null;
  })) as number[] | null;
  if (!box) throw new Error("no status field");
  const served = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.style.setProperty("visibility", "hidden"), selector);
  const hidden = decodePng(Buffer.from(await browser.takeScreenshot(), "base64"));
  await browser.execute((sel: string) => document.querySelector<HTMLElement>(sel)?.style.removeProperty("visibility"), selector);
  const scale = served.width / box[4];
  const [x, y, w, h] = box.slice(0, 4).map((v) => Math.round(v * scale));
  let changed = 0;
  for (let row = Math.max(0, y); row < Math.min(served.height, y + h); row++) {
    for (let col = Math.max(0, x); col < Math.min(served.width, x + w); col++) {
      const i = (row * served.width + col) * 4;
      if (Math.abs(served.data[i] - hidden.data[i]) + Math.abs(served.data[i + 1] - hidden.data[i + 1]) + Math.abs(served.data[i + 2] - hidden.data[i + 2]) > 48) changed++;
    }
  }
  return changed;
}

/** Every built-in pack that lays the buddy list's status field. */
const PACKS = ["symbiosis", "combat", "onyx", "alloy", "xslimfire", "simple-x", "total-chaos", "xfire-classic"];

describe("a paused inbound route is said in the tree's status field, which stays usable", function () {
  this.timeout(600_000);
  const me: User = newUser("paused");

  before(async () => {
    await createIdentity(browser, me);
    await switchTo(browser, BUDDY);
    if (!(await underTree(browser))) throw new Error("the default skin no longer lays the buddy list as a tree");
  });

  it("the status field says the Host's cause, whole, as an alert", async () => {
    const line = await shown(browser, PAUSED, NET_WAIT_MS);
    expect((await line.getText()).trim()).toBe(EXHAUSTED);
    // The whole line is the field's tooltip too: the pack's box ellipsizes it.
    expect(await (await browser.$(FIELD)).getAttribute("title")).toBe(EXHAUSTED);
  });

  // Lead (RV1): the paused line replaces the status text in the field, in
  // every pack; one text is seen there, never both (Combat drew the
  // input's "Online" through the line).
  for (const file of PACKS) {
    it(`${file}: only the paused line is seen in the status field, the input's text under it`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      // Settings closes after each switch, so the next pack's opens fresh
      // and frontmost (left open, its Appearance tab took no click).
      await switchTo(browser, SETTINGS);
      await closeSettings(browser);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      // Evidence first (a pack that shows no line): the status node's and
      // the line's boxes and stacking, and what is on top at the line.
      const probe = (await browser.execute(() => {
        const box = (e: Element | null): string => {
          if (!e) return "none";
          const r = e.getBoundingClientRect();
          const cs = getComputedStyle(e);
          return `${Math.round(r.x)},${Math.round(r.y)} ${Math.round(r.width)}x${Math.round(r.height)} z=${cs.zIndex} pos=${cs.position} vis=${cs.visibility} op=${cs.opacity}`;
        };
        const field = document.querySelector(".skin-status");
        const line = document.querySelector(".skin-status-paused");
        const r = line?.getBoundingClientRect();
        const top = r ? (document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) as HTMLElement | null) : null;
        const art = (e: Element | null): string => (e ? getComputedStyle(e).backgroundImage.slice(0, 80) : "-");
        const statusNodes = Array.from(document.querySelectorAll<HTMLElement>('.skin-tree [data-skin-role^="status"], .skin-tree [data-skin-role^="name"], .skin-tree [data-skin-role*="presence" i]')).map(
          (n) => `${n.dataset.skinNode} (${n.dataset.skinRole}) ${box(n)} art=${art(n)}`,
        );
        const fieldEl = field as HTMLElement | null;
        return JSON.stringify({
          field: box(field),
          fieldNode: `${fieldEl?.dataset.skinNode ?? "none"} (${fieldEl?.dataset.skinRole ?? "-"}) art=${art(field)}`,
          line: box(line),
          atLine: top ? `${top.tagName.toLowerCase()}.${top.className} in ${top.closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode ?? "none"}` : "none",
          statusNodes,
        });
      })) as string;
      console.log(`[paused] ${file} probe: ${probe}`);
      await shown(browser, PAUSED, NET_WAIT_MS);
      await browser.pause(1000);
      const line = await seenInField(PAUSED);
      const input = await seenInField(FIELD);
      console.log(`[paused] ${file}: hiding the line changes ${line} px, hiding the input ${input} px (one text: line >= 20, input < 20)`);
      // Lead (Alloy, 15 px): a line not seen, measured before anything
      // changes: every element at its centre, topmost first, with its
      // stacking and paint, and the line's text colour against the ground
      // under it.
      if (line < 20) {
        const stack = (await browser.execute(() => {
          const l = document.querySelector<HTMLElement>(".skin-status-paused");
          if (!l) return "no line";
          const r = l.getBoundingClientRect();
          const cs = getComputedStyle(l);
          const at = document.elementsFromPoint(r.x + r.width / 2, r.y + r.height / 2).slice(0, 8).map((e) => {
            const c = getComputedStyle(e);
            const node = (e as HTMLElement).closest<HTMLElement>("[data-skin-node]")?.dataset.skinNode ?? "-";
            return `${e.tagName.toLowerCase()}.${(e as HTMLElement).className || "-"} in ${node} z=${c.zIndex} pos=${c.position} bg=${c.backgroundColor} art=${c.backgroundImage === "none" ? "none" : c.backgroundImage.split("/").pop()?.slice(0, 24)} op=${c.opacity}`;
          });
          return JSON.stringify({ lineColour: cs.color, lineBg: cs.backgroundColor, lineFont: `${cs.fontSize} ${cs.fontWeight}`, lineText: (l.textContent ?? "").slice(0, 40), at });
        })) as string;
        console.log(`[paused] ${file} not seen; stack at the line: ${stack}`);
      }
      await shot(`${file}-paused-field`);
      expect({ pack: file, lineSeen: line >= 20, inputSeen: input >= 20 }).toEqual({ pack: file, lineSeen: true, inputSeen: false });
    });
  }

  it("the away message can still be typed: the line steps aside while the field has focus, then returns", async () => {
    const field = await browser.$(FIELD);
    await field.click();
    await gone(browser, PAUSED, NET_WAIT_MS);
    const away = "back after the reboot";
    await field.setValue(away);
    await browser.keys(["Enter"]);
    await browser.execute(() => (document.activeElement as HTMLElement | null)?.blur());
    await shown(browser, PAUSED, NET_WAIT_MS);
    // Set, not dropped: the field shows the Host's stored message after blur.
    await waitUntil(async () => (await field.getValue()) === away, NET_WAIT_MS, "the away message typed while paused was not kept");
  });
});
