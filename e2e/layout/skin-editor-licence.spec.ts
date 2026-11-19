import { expect, test } from "@playwright/test";
import { FULL } from "../fixtures/stress";
import { WINDOWS, open, sizes } from "./windows";

// Plan E7.7.5, the Linux walk (2026-11-18): the licence textarea clipped
// its text mid-line at the editor's default size. A licence is often a
// full text, so the field takes the whole row and shows at least six
// whole lines: its content box is a whole number of lines tall.

const EDITOR = WINDOWS.find((w) => w.name === "skin-editor")!;
const MIN_LINES = 6;

for (const [sizeName, size] of sizes(EDITOR).filter(([n]) => n !== "large")) {
  test(`skin-editor licence ${sizeName} ${size.width}x${size.height}`, async ({ page }) => {
    await open(page, EDITOR, size, FULL);
    const licence = page.getByLabel("Licence", { exact: true });
    const m = await licence.evaluate((el) => {
      const s = getComputedStyle(el);
      const meta = el.closest(".skin-editor-meta")!.getBoundingClientRect();
      return {
        content: el.clientHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom),
        line: parseFloat(s.lineHeight),
        width: el.getBoundingClientRect().width,
        row: meta.width,
      };
    });
    expect(Number.isFinite(m.line), "a set line height").toBe(true);
    const lines = m.content / m.line;
    expect(lines, `${m.content}px over ${m.line}px lines`).toBeGreaterThanOrEqual(MIN_LINES);
    expect(Math.abs(lines - Math.round(lines)) * m.line, "no line cut at the bottom").toBeLessThan(1);
    expect(m.width, "the licence takes the whole row").toBeGreaterThan(m.row - 1);
  });
}
