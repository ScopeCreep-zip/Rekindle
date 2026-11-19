import type { Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Plan E7.3 "Gate": the rule set is WCAG 2.0–2.2 A and AA, the level the
// plan targets (wiring guide §13).
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

/** Each axe violation on `page`: its rule id, impact and offending nodes. */
export async function violations(page: Page) {
  const result = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  return result.violations.map((v) => ({
    rule: v.id,
    impact: v.impact,
    nodes: v.nodes.map((n) => n.target.join(" ")).slice(0, 8),
  }));
}
