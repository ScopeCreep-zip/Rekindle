// Plan CR1: a disabled node says why, never a reason-less one. In each pack's
// buddy list with nothing selected (so selection-dependent controls are
// disabled), every drawn node the tree marks disabled (`aria-disabled`)
// carries the Host's reason as its `aria-description` and `title`; one the
// pack draws no disabled art for (`data-disabled-by="shell"`) is dimmed, and
// one with its own art (`pack`) is not. Each node logged as `[cr1]` and shot.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

const PACKS = ["symbiosis", "onyx", "combat", "alloy", "xslimfire", "simple-x"];

interface Disabled {
  node: string;
  reason: string;
  title: string;
  by: string;
  opacity: string;
}

describe("a disabled node says why (CR1)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("cr1"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
  });

  for (const file of PACKS) {
    it(`${file}: every disabled node carries the Host's reason; only shell-drawn ones are dimmed`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      await browser.pause(2000);
      const nodes = (await browser.execute(() =>
        Array.from(document.querySelectorAll<HTMLElement>('[data-skin-node][aria-disabled="true"]'))
          .filter((n) => !n.closest("[hidden]") && n.getBoundingClientRect().width > 0)
          .map((n) => ({
            node: n.dataset.skinNode ?? "",
            reason: n.getAttribute("aria-description") ?? "",
            title: n.getAttribute("title") ?? "",
            by: n.dataset.disabledBy ?? "",
            opacity: getComputedStyle(n).opacity,
          })),
      )) as Disabled[];
      for (const d of nodes) console.log(`[cr1] ${file} ${d.node}: by=${d.by || "?"} opacity=${d.opacity} reason="${d.reason}"`);
      await shot(`${file}-disabled`);
      expect(nodes.length).toBeGreaterThan(0);
      expect(nodes.filter((d) => d.reason.trim() === "").map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.title !== d.reason).map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.by === "shell" && d.opacity !== "0.5").map((d) => d.node)).toEqual([]);
      expect(nodes.filter((d) => d.by === "pack" && d.opacity !== "1").map((d) => d.node)).toEqual([]);
    });
  }
});
