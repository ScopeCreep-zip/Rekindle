import { test } from "@playwright/test";
import {
  assertFlexContract,
  assertFocusVisibleNotClipped,
  assertNoHorizontalOverflow,
  assertNoOverlap,
  assertSpacingOnScale,
  assertTextFits,
} from "../fixtures/layout";
import { EMPTY, FULL, type Stress } from "../fixtures/stress";
import { WINDOWS, open, sizes } from "./windows";

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
        await assertSpacingOnScale(page, root, w.spacing);
        await assertFocusVisibleNotClipped(page, root);
      });
    }
  }
}
