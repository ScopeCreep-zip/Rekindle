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

// Plan E7.7.2 "Apply": the region rules under the user's settings
// (`src/styles/generated/regions-a11y.css`, `@layer a11y`). The skin half
// is written here in the shape `rekindle_skin::regions::css::skin_css`
// emits (`@layer skin { :root { … } }`, a `--still` beside an animated
// image); the rules under test are the generated app CSS the page loads.
// Each case first checks the media query matches, and the control case
// checks it does not, so an emulation that did nothing fails the case
// instead of passing it.
const IMAGE = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==";
const STILL = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==";
const SKIN = `@layer skin { :root {
  --skin-region-titlebar-btn-minimize: url("${IMAGE}") 0 fill / 0 / 0 stretch;
  --skin-region-titlebar-btn-minimize--still: url("${STILL}") 0 fill / 0 / 0 stretch;
} }`;

const A11Y_CASES: {
  name: string;
  media: Parameters<Page["emulateMedia"]>[0];
  query: string | null;
  source: (s: string) => boolean;
}[] = [
  { name: "no preference", media: {}, query: null, source: (s) => s.includes("image/gif") },
  {
    name: "reduced motion shows the still",
    media: { reducedMotion: "reduce" },
    query: "(prefers-reduced-motion: reduce)",
    source: (s) => s.includes("image/png"),
  },
  {
    name: "forced colours drop the image",
    media: { forcedColors: "active" },
    query: "(forced-colors: active)",
    source: (s) => s === "none",
  },
];

for (const c of A11Y_CASES) {
  test(`skin regions under ${c.name}`, async ({ page }) => {
    const w = WINDOWS.find((x) => x.name === "buddy-list");
    if (w === undefined) throw new Error("no buddy-list window case");
    await page.emulateMedia(c.media);
    await open(page, w, w.def, FULL);
    await page.addStyleTag({ content: SKIN });
    const matches = await page.evaluate(
      () =>
        [
          matchMedia("(prefers-reduced-motion: reduce)").matches,
          matchMedia("(forced-colors: active)").matches,
        ] as const,
    );
    expect(matches).toEqual([
      c.query === "(prefers-reduced-motion: reduce)",
      c.query === "(forced-colors: active)",
    ]);
    const source = await page
      .locator(".xfire-titlebar-btn-minimize")
      .first()
      .evaluate((el) => getComputedStyle(el).borderImageSource);
    expect(c.source(source), source).toBe(true);
  });
}
