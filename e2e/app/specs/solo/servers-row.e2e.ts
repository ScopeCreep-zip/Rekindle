// Lead (finding SR1, "every Xfire button works in every layout"): the
// Servers view's own button row on its page. Alloy's and Simple X's Servers
// slots sit in group-1 (views group) wrappers, as the cloned Communities
// slots did before CB2, so `offPage` may hide them on the Servers page. For
// each pack: the Servers view shown, then every node of its
// `buttonRow:servers` with whether it is drawn and, when hidden, the nearest
// hidden ancestor (`[sr]`), with a shot of the window. Required: the row
// and at least one of its buttons drawn.

import { browser, expect } from "@wdio/globals";
import { switchTo } from "../../../walk/helpers.ts";
import { BUDDY, openSettings } from "../../../walk/nav.ts";
import { createIdentity, newUser, type User } from "../../journeys.ts";
import { applySkin, showSkinView, skinShown } from "../../journeys_skins.ts";
import { shot } from "../../shots.ts";
import { builtinPacks } from "../../skin-pack.ts";

/** Each pack's view whose own button row is checked (SR1: Alloy, Simple X
 *  and XGlow's Servers rows, Combat's Servers and Friends rows; Symbiosis's
 *  Servers row as CB2 left it). */
const CASES = [
  { file: "alloy", view: "servers" },
  { file: "simple-x", view: "servers" },
  { file: "xglow", view: "servers" },
  { file: "combat", view: "servers" },
  { file: "combat", view: "friends" },
  { file: "symbiosis", view: "servers" },
] as const;

interface RowReport {
  row: boolean;
  nodes: string[];
  drawnButtons: number;
  /** Each button's box and whether it is visible: inside the window, not
   *  zero-sized, not transparent or hidden, and the top element at its
   *  centre (lead: "drawn" alone did not prove the shot shows it). */
  visible: { node: string; rect: number[]; seen: boolean; why: string }[];
  /** The selected tab's word (the tab row's pressed/active node), to check
   *  the view's tab is the highlighted one. */
  communitiesWordColor: string;
}

describe("a view's own button row shows on its page (SR1)", function () {
  this.timeout(1_200_000);
  const me: User = { ...newUser("srrow"), displayName: "Ari" };

  before(async () => {
    await createIdentity(browser, me);
  });

  for (const { file, view } of CASES) {
    it(`${file}: the ${view} row and its buttons are drawn in the ${view} view`, async () => {
      const pack = builtinPacks().find((p) => p.file === file);
      if (!pack) throw new Error(`no built-in pack ${file}`);
      await switchTo(browser, BUDDY);
      await openSettings(browser, "Appearance");
      const href = await applySkin(browser, me, pack.name);
      await switchTo(browser, BUDDY);
      await skinShown(browser, href);
      await showSkinView(browser, view);
      await browser.pause(1000);
      const report = (await browser.execute((v: string) => {
        const drawn = (e: Element): boolean => !e.closest("[hidden]") && e.getBoundingClientRect().width > 0;
        const rows = Array.from(document.querySelectorAll<HTMLElement>(`[data-skin-role="buttonRow:${v}"]`));
        const row = rows.find(drawn) ?? rows[0];
        if (!row) return { row: false, nodes: [], drawnButtons: 0 };
        const nodes = Array.from(row.querySelectorAll<HTMLElement>("[data-skin-node]")).map((n) => {
          const hiddenBy = n.closest<HTMLElement>("[hidden]")?.dataset.skinNode;
          return `${n.dataset.skinNode}(${n.dataset.skinRole}) ${drawn(n) ? "drawn" : `hidden${hiddenBy ? ` by ${hiddenBy}` : ""}`}`;
        });
        const drawnButtons = Array.from(row.querySelectorAll<HTMLElement>("[data-skin-node]")).filter(
          (n) => drawn(n) && (n.dataset.skinRole === "button" || n.getAttribute("role") === "button"),
        ).length;
        const visible = Array.from(row.querySelectorAll<HTMLElement>("[data-skin-node]"))
          .filter((n) => n.dataset.skinRole === "button" || n.getAttribute("role") === "button")
          .map((n) => {
            const r = n.getBoundingClientRect();
            const cs = getComputedStyle(n);
            const cx = r.x + r.width / 2;
            const cy = r.y + r.height / 2;
            const top = document.elementFromPoint(cx, cy);
            const why =
              r.width <= 0 || r.height <= 0
                ? "zero-sized"
                : r.right <= 0 || r.bottom <= 0 || r.left >= window.innerWidth || r.top >= window.innerHeight
                  ? "outside the window"
                  : cs.visibility === "hidden" || Number(cs.opacity) === 0
                    ? "transparent or hidden"
                    : !top || !(top === n || n.contains(top))
                      ? `covered by ${(top as HTMLElement | null)?.dataset?.skinNode ?? top?.tagName ?? "nothing"}`
                      : "";
            return { node: n.dataset.skinNode ?? "", rect: [r.x, r.y, r.width, r.height].map(Math.round), seen: why === "", why };
          });
        const comm = Array.from(document.querySelectorAll<HTMLElement>(".skin-node-label-text")).find((t) => (t.textContent ?? "").trim() === "Communities");
        return { row: drawn(row), nodes, drawnButtons, visible, communitiesWordColor: comm ? getComputedStyle(comm).color : "none" };
      }, view)) as RowReport;
      console.log(`[sr] ${file} ${view}: ${JSON.stringify(report)}`);
      await shot(`${view}-view`);
      expect(report.row).toBe(true);
      expect(report.drawnButtons).toBeGreaterThan(0);
      expect(report.visible.filter((v) => !v.seen)).toEqual([]);
    });
  }
});
