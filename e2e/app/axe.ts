// Plan E7.14 S5 (moved from E7.3's e2e/a11y/axe.ts): axe in the real
// window, on the engine users run. axe-core itself is injected (its
// exported `source`) and run through WebDriver's execute, whose script the
// driver awaits (vendor exec_script.rs). `@axe-core/webdriverio` is not
// used: 4.14.0 declares webdriverio "^5 || ... || ^9" (we run 10), and what
// it adds is per-frame injection, while the app's windows hold no content
// frames.

import axe from "axe-core";
import type { Browser } from "webdriverio";

// Plan E7.3 "Gate": the rule set is WCAG 2.0–2.2 A and AA, the level the
// plan targets (wiring guide §13).
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

export interface Violation {
  rule: string;
  impact: string | null | undefined;
  nodes: string[];
}

/** Each axe violation in the window `b` is in: its rule id, impact and offending nodes. */
export async function violations(b: Browser): Promise<Violation[]> {
  const loaded = await b.execute(() => typeof (window as unknown as { axe?: unknown }).axe === "object");
  if (!loaded) await b.execute(`${axe.source}; return true;`);
  const script = `return window.axe
  .run(document, { runOnly: { type: "tag", values: ${JSON.stringify(TAGS)} }, resultTypes: ["violations"] })
  .then(function (r) {
    return r.violations.map(function (v) {
      // Every undersized target, so each is named (plan SK7); other rules keep eight.
      return { rule: v.id, impact: v.impact, nodes: v.nodes.map(function (n) { return n.target.join(" "); }).slice(0, v.id === "target-size" ? 500 : 8) };
    });
  });`;
  return (await b.execute(script)) as Violation[];
}
