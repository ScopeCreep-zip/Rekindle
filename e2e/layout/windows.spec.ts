import { test } from "@playwright/test";
import {
  assertFlexContract,
  assertFieldsUsable,
  assertFocusVisibleNotClipped,
  assertNoHorizontalOverflow,
  assertBadgesInRow,
  assertNoOverlap,
  assertNoPlaceholderText,
  assertPanesNoHorizontalScroll,
  assertSpacingOnScale,
  assertTextFits,
  assertTextNotClipped,
  assertTimestampsInline,
} from "../fixtures/layout";
import { EMPTY, FULL, type Stress } from "../fixtures/stress";
import { WINDOWS, open, panesOf, sizes, spacingOf } from "./windows";

// Plan E7.12: every routed window at its minimum, default and a large size,
// over stress data (and, where a window has one, its empty state). The
// geometry checks are the pass/fail; the screenshot is evidence.

const DATA: [string, Stress][] = [["stress", FULL], ["empty", EMPTY]];

for (const w of WINDOWS) {
  for (const [sizeName, size] of sizes(w)) {
    for (const [dataName, data] of DATA) {
      test(`${w.name} ${sizeName} ${size.width}x${size.height} ${dataName}`, async ({ page }, testInfo) => {
        await open(page, w, size, data);
        await testInfo.attach(`${w.name}-${sizeName}-${dataName}.png`, {
          body: await page.screenshot(),
          contentType: "image/png",
        });
        const root = page.locator(".app-frame").first();
        await assertFlexContract(page, root);
        await assertNoHorizontalOverflow(page, root);
        await assertNoOverlap(page, root);
        await assertTextFits(page, root);
        await assertTextNotClipped(page, root);
        await assertPanesNoHorizontalScroll(page, root, panesOf(w, data !== EMPTY));
        await assertBadgesInRow(page, root);
        await assertNoPlaceholderText(page, root);
        await assertTimestampsInline(page, root);
        await assertFieldsUsable(page, root);
        await assertSpacingOnScale(page, root, spacingOf(w, data !== EMPTY));
        await assertFocusVisibleNotClipped(page, root);
      });
    }
  }
}
