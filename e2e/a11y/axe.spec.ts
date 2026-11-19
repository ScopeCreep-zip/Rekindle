import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { FULL } from "../fixtures/stress";
import { WINDOWS, open } from "../layout/windows";

// Plan E7.3 "Gate": axe-core over every routed window, at its default size
// over the layout suite's stress data, under the token skin's dark and light
// bases. The rule set is WCAG 2.0–2.2 A and AA, the level the plan targets
// (wiring guide §13). A violation fails the window's test with its rule id,
// impact and the offending nodes.

const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

for (const base of ["dark", "light"] as const) {
  for (const w of WINDOWS) {
    test(`${w.name} ${base}`, async ({ page }) => {
      await open(page, w, w.def, FULL, base);
      const result = await new AxeBuilder({ page }).withTags(TAGS).analyze();
      const found = result.violations.map((v) => ({
        rule: v.id,
        impact: v.impact,
        nodes: v.nodes.map((n) => n.target.join(" ")).slice(0, 8),
      }));
      expect(found, `${w.name} (${base}) axe violations`).toEqual([]);
    });
  }
}
