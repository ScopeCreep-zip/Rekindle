import { expect, test, type Page } from "@playwright/test";
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
import { FULL } from "../fixtures/stress";
import { WINDOWS, open, panesOf, spacingOf } from "./windows";

// Plan E7.12 "Skins": the geometry set again under the light base (the
// apply step's `:root[data-skin-base]`), forced colours and more contrast,
// each window at its default size over the stress data.

interface Variant {
  name: string;
  base: "dark" | "light";
  media: Parameters<Page["emulateMedia"]>[0];
}

const VARIANTS: Variant[] = [
  { name: "light", base: "light", media: {} },
  { name: "forced-colors", base: "dark", media: { forcedColors: "active" } },
  { name: "contrast-more", base: "dark", media: { contrast: "more" } },
];

for (const v of VARIANTS) {
  for (const w of WINDOWS) {
    test(`${w.name} ${v.name}`, async ({ page }, testInfo) => {
      await page.emulateMedia(v.media);
      await open(page, w, w.def, FULL, v.base);
      await expect(page.locator("html")).toHaveAttribute("data-skin-base", v.base);
      await testInfo.attach(`${w.name}-${v.name}.png`, {
        body: await page.screenshot(),
        contentType: "image/png",
      });
      const root = page.locator(".app-frame").first();
      await assertFlexContract(page, root);
      await assertNoHorizontalOverflow(page, root);
      await assertNoOverlap(page, root);
      await assertTextFits(page, root);
      await assertTextNotClipped(page, root);
      await assertPanesNoHorizontalScroll(page, root, panesOf(w, true));
      await assertBadgesInRow(page, root);
      await assertNoPlaceholderText(page, root);
      await assertTimestampsInline(page, root);
      await assertFieldsUsable(page, root);
      await assertSpacingOnScale(page, root, spacingOf(w, true));
      await assertFocusVisibleNotClipped(page, root);
    });
  }
}
