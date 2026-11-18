import { expect, test, type Page } from "@playwright/test";
import { FULL } from "../fixtures/stress";
import { WINDOWS, open, type WindowCase } from "./windows";

// Plan E7.12 / E7.13 (Linux finding): WebKitGTK drew the native <select>
// as a light Adwaita box under a dark skin, its text unreadable. Every
// select is skin-drawn now (`91-top-level.css`: `appearance: none`,
// `color-scheme` from the skin's base, themed options). Playwright's WebKit
// cannot reproduce GTK's native drawing, so this asserts the computed CSS:
// the select and each option paint an opaque skin ground with text at
// WCAG 1.4.3's 4.5:1, under the dark and the light built-in base.

type Rgba = [number, number, number, number];

/** `rgb()`, `rgba()` or `color(srgb …)` as computed, channels 0-1. */
function parse(css: string): Rgba {
  const srgb = css.match(/^color\(srgb ([\d.e-]+) ([\d.e-]+) ([\d.e-]+)(?: \/ ([\d.e-]+))?\)$/);
  if (srgb) return [Number(srgb[1]), Number(srgb[2]), Number(srgb[3]), srgb[4] ? Number(srgb[4]) : 1];
  const rgb = css.match(/^rgba?\(([\d.]+), ([\d.]+), ([\d.]+)(?:, ([\d.]+))?\)$/);
  if (rgb) return [Number(rgb[1]) / 255, Number(rgb[2]) / 255, Number(rgb[3]) / 255, rgb[4] ? Number(rgb[4]) : 1];
  throw new Error(`unparsed colour ${css}`);
}

function luminance([r, g, b]: Rgba): number {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(fg: Rgba, bg: Rgba): number {
  const [a, b] = [luminance(fg), luminance(bg)].sort((x, y) => y - x);
  return (a + 0.05) / (b + 0.05);
}

interface Painted {
  color: string;
  background: string;
  appearance: string;
  scheme: string;
}

async function painted(page: Page, selector: string): Promise<Painted[]> {
  return page.locator(selector).evaluateAll((els) =>
    els.map((el) => {
      const s = getComputedStyle(el);
      return {
        color: s.color,
        background: s.backgroundColor,
        appearance: s.appearance,
        scheme: s.colorScheme,
      };
    }),
  );
}

const settings = WINDOWS.find((x) => x.name === "settings");
if (settings === undefined) throw new Error("no settings window case");
const APPLICATION: WindowCase = {
  ...settings,
  name: "settings-application",
  path: "/settings?tab=application",
  ready: async (page) => {
    await settings.ready(page);
    await page.locator("select.form-select").first().waitFor();
  },
};

for (const base of ["dark", "light"] as const) {
  test(`select contrast ${base}`, async ({ page }) => {
    await open(page, APPLICATION, APPLICATION.def, FULL, base);
    await expect(page.locator("html")).toHaveAttribute("data-skin-base", base);

    const selects = await painted(page, "select.form-select");
    expect(selects.length).toBeGreaterThan(0);
    for (const s of selects) {
      expect(s.appearance, "the skin draws the select").toBe("none");
      expect(s.scheme, "native parts follow the skin's base").toBe(base);
      const bg = parse(s.background);
      expect(bg[3], `select ground ${s.background} is opaque`).toBe(1);
      const ratio = contrast(parse(s.color), bg);
      expect(ratio, `select ${s.color} on ${s.background}`).toBeGreaterThanOrEqual(4.5);
    }

    const options = await painted(page, "select.form-select option");
    expect(options.length).toBeGreaterThan(0);
    for (const o of options) {
      expect(o.scheme, "the option popup follows the skin's base").toBe(base);
      const bg = parse(o.background);
      expect(bg[3], `option ground ${o.background} is opaque`).toBe(1);
      const ratio = contrast(parse(o.color), bg);
      expect(ratio, `option ${o.color} on ${o.background}`).toBeGreaterThanOrEqual(4.5);
    }
  });
}
